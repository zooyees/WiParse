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
                capabilities.channels = 1;
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
    let kind = requested.unwrap_or_else(|| classify_instrument_kind(&vendor, &model));
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
    if kind == InstrumentKind::Oscilloscope
        && (vendor.contains("RIGOL") || vendor.contains("SIGLENT") || vendor.contains("KEYSIGHT"))
    {
        profile.capabilities.screenshot = true;
    }
    if kind == InstrumentKind::DcSource {
        profile.capabilities.channels = estimate_dc_source_channels(&model);
    }
    profile
}

/// Infer DC supply channel count from the `*IDN?` model string.
pub fn estimate_dc_source_channels(model: &str) -> u8 {
    let m = model
        .to_uppercase()
        .replace(['-', ' ', '_'], "");
    const KNOWN: &[(&str, u8)] = &[
        ("DP832", 3),
        ("DP831", 3),
        ("DP932", 3),
        ("DP821", 2),
        ("DP822", 2),
        ("DP811", 2),
        ("DP711", 1),
        ("DP712", 1),
        ("E3631", 3),
        ("E3632", 1),
        ("E3633", 1),
        ("E3634", 1),
        ("IT6302", 3),
        ("IT6322", 3),
        ("IT6332", 3),
        ("IT6333", 3),
        ("N6705", 4),
        ("N6700", 4),
    ];
    for (key, channels) in KNOWN {
        if m.contains(key) {
            return *channels;
        }
    }
    // Unknown models: stay at 1 until a confirmed channel count is available.
    1
}

/// Infer instrument class from `*IDN?` manufacturer/model fields.
pub fn classify_instrument_kind(vendor: &str, model: &str) -> InstrumentKind {
    let vendor = vendor.to_uppercase();
    let model = model.to_uppercase();
    if vendor.contains("TEKTRONIX")
        || vendor.contains("SIGLENT")
        || model.starts_with("MDO")
        || model.starts_with("MSO")
        || model.starts_with("DPO")
        || model.starts_with("TBS")
        || model.starts_with("TDS")
        || model.starts_with("DSO")
        || model.starts_with("DS1")
        || model.starts_with("DS2")
        || model.starts_with("DS4")
        || model.starts_with("DS6")
        || model.starts_with("DS7")
        || model.starts_with("SDS")
        || model.contains("OSCILLOSCOPE")
        || (vendor.contains("RIGOL") && (model.starts_with("DS") || model.starts_with("MSO")))
    {
        InstrumentKind::Oscilloscope
    } else if model.starts_with("DP")
        || model.starts_with("E36")
        || model.starts_with("N67")
        || model.starts_with("N57")
        || model.starts_with("PSU")
        || model.contains("SOURCE")
        || model.contains("POWER SUPPLY")
        || (vendor.contains("ITECH") && model.starts_with("IT63"))
        || (vendor.contains("RIGOL") && model.starts_with("DP"))
    {
        InstrumentKind::DcSource
    } else if model.starts_with("IT8")
        || model.starts_with("PLZ")
        || model.starts_with("N33")
        || model.contains("LOAD")
        || model.contains("ELECTRONIC LOAD")
    {
        InstrumentKind::ElectronicLoad
    } else if vendor.contains("KEITHLEY")
        || model.starts_with("34")
        || model.starts_with("DM")
        || model.contains("DMM")
        || model.contains("MULTIMETER")
        || (vendor.contains("KEYSIGHT") && (model.starts_with("344") || model.starts_with("345")))
    {
        InstrumentKind::Multimeter
    } else {
        InstrumentKind::Generic
    }
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
    pub fn all() -> &'static [Self] {
        &[
            Self::DcVoltage,
            Self::AcVoltage,
            Self::DcCurrent,
            Self::AcCurrent,
            Self::Resistance,
            Self::Frequency,
        ]
    }

    pub fn scpi(self) -> &'static str {
        match self {
            Self::DcVoltage => "VOLT:DC",
            Self::AcVoltage => "VOLT:AC",
            Self::DcCurrent => "CURR:DC",
            Self::AcCurrent => "CURR:AC",
            Self::Resistance => "RES",
            Self::Frequency => "FREQ",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::DcVoltage => "DC V",
            Self::AcVoltage => "AC V",
            Self::DcCurrent => "DC A",
            Self::AcCurrent => "AC A",
            Self::Resistance => "Ω",
            Self::Frequency => "Hz",
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

/// Tektronix-style immediate measurement types (MEASUrement:IMMed:TYPe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ScopeMeasType {
    #[default]
    Frequency,
    Period,
    Pk2Pk,
    Amplitude,
    Maximum,
    Minimum,
    Mean,
    Rms,
    Rise,
    Fall,
    PosWidth,
    NegWidth,
    DutyCycle,
}

impl ScopeMeasType {
    pub fn all() -> &'static [Self] {
        &[
            Self::Frequency,
            Self::Period,
            Self::Pk2Pk,
            Self::Amplitude,
            Self::Maximum,
            Self::Minimum,
            Self::Mean,
            Self::Rms,
            Self::Rise,
            Self::Fall,
            Self::PosWidth,
            Self::NegWidth,
            Self::DutyCycle,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Frequency => "Frequency",
            Self::Period => "Period",
            Self::Pk2Pk => "Peak-Peak",
            Self::Amplitude => "Amplitude",
            Self::Maximum => "Maximum",
            Self::Minimum => "Minimum",
            Self::Mean => "Mean",
            Self::Rms => "RMS",
            Self::Rise => "Rise",
            Self::Fall => "Fall",
            Self::PosWidth => "+Width",
            Self::NegWidth => "-Width",
            Self::DutyCycle => "Duty",
        }
    }

    pub fn scpi(self) -> &'static str {
        match self {
            Self::Frequency => "FREQuency",
            Self::Period => "PERIod",
            Self::Pk2Pk => "PK2PK",
            Self::Amplitude => "AMPlitude",
            Self::Maximum => "MAXIMUM",
            Self::Minimum => "MINImum",
            Self::Mean => "MEAN",
            Self::Rms => "RMS",
            Self::Rise => "RISe",
            Self::Fall => "FALL",
            Self::PosWidth => "PWIdth",
            Self::NegWidth => "NWIdth",
            Self::DutyCycle => "PDUTy",
        }
    }

    /// Base SI unit used when the instrument omits UNIts.
    pub fn base_unit(self) -> &'static str {
        match self {
            Self::Frequency => "Hz",
            Self::Period | Self::Rise | Self::Fall | Self::PosWidth | Self::NegWidth => "s",
            Self::Pk2Pk
            | Self::Amplitude
            | Self::Maximum
            | Self::Minimum
            | Self::Mean
            | Self::Rms => "V",
            Self::DutyCycle => "%",
        }
    }
}

fn format_scope_measure_payload(raw: &str, meas_type: ScopeMeasType, unit: &str) -> String {
    let (num_part, unit_from_value) = split_number_and_unit(raw.trim());
    let unit = if unit.trim().is_empty() {
        unit_from_value
    } else {
        unit.trim()
    };
    if let Some(value) = parse_f64_loose(num_part) {
        let (base_value, base_unit) = value_to_base_unit(value, unit, meas_type);
        format_human_scope_reading(base_value, meas_type, base_unit)
    } else {
        humanize_scope_reading_text(raw, meas_type)
    }
}

/// Format a scope measurement for humans: auto SI prefix, no scientific notation.
///
/// Examples: `99.9 MHz`, `12.5 µs`, `3.30 V`, `48.2 %`.
/// `unit` may be base (`Hz`/`s`/`V`) or already prefixed (`MHz`/`µs`); values are
/// normalized to base SI before scaling.
pub fn format_human_scope_reading(value: f64, meas_type: ScopeMeasType, unit: &str) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    let (value, unit) = value_to_base_unit(value, unit, meas_type);
    match unit {
        "Hz" => format_si_scaled(value, &[("GHz", 1e9), ("MHz", 1e6), ("kHz", 1e3), ("Hz", 1.0)]),
        "s" => format_si_scaled(
            value,
            &[
                ("s", 1.0),
                ("ms", 1e-3),
                ("µs", 1e-6),
                ("ns", 1e-9),
                ("ps", 1e-12),
            ],
        ),
        "V" => format_si_scaled(value, &[("V", 1.0), ("mV", 1e-3), ("µV", 1e-6)]),
        "A" => format_si_scaled(value, &[("A", 1.0), ("mA", 1e-3), ("µA", 1e-6)]),
        "%" => format!("{} %", format_plain_number(value)),
        other => format!("{} {}", format_plain_number(value), other),
    }
}

/// Re-format a raw instrument fragment like `9.99E+07 Hz` or `1.25e-6`.
pub fn humanize_scope_reading_text(text: &str, meas_type: ScopeMeasType) -> String {
    let text = text.trim();
    if text.is_empty() || text == "—" {
        return text.to_owned();
    }
    let (num_part, unit_part) = split_number_and_unit(text);
    let Some(value) = parse_f64_loose(num_part) else {
        return text.to_owned();
    };
    // Convert already-prefixed units back to base SI before re-scaling.
    let (base_value, unit) = value_to_base_unit(value, unit_part, meas_type);
    format_human_scope_reading(base_value, meas_type, unit)
}

fn value_to_base_unit(value: f64, unit: &str, meas_type: ScopeMeasType) -> (f64, &'static str) {
    let u = unit.trim().trim_matches('"').trim_matches('\'');
    let lower = u.to_ascii_lowercase().replace('μ', "µ");
    match lower.as_str() {
        "ghz" => (value * 1e9, "Hz"),
        "mhz" => (value * 1e6, "Hz"),
        "khz" => (value * 1e3, "Hz"),
        "hz" | "hertz" | "" if meas_type.base_unit() == "Hz" => (value, "Hz"),
        "s" | "sec" | "second" | "seconds" => (value, "s"),
        "ms" => (value * 1e-3, "s"),
        "us" | "µs" => (value * 1e-6, "s"),
        "ns" => (value * 1e-9, "s"),
        "ps" => (value * 1e-12, "s"),
        "v" | "volt" | "volts" => (value, "V"),
        "mv" => (value * 1e-3, "V"),
        "uv" | "µv" => (value * 1e-6, "V"),
        "a" => (value, "A"),
        "ma" => (value * 1e-3, "A"),
        "ua" | "µa" => (value * 1e-6, "A"),
        "%" | "pct" | "percent" => (value, "%"),
        _ => (value, meas_type.base_unit()),
    }
}

fn split_number_and_unit(text: &str) -> (&str, &str) {
    let bytes = text.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    let (num, rest) = text.split_at(i);
    (num.trim(), rest.trim())
}

fn parse_f64_loose(text: &str) -> Option<f64> {
    text.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .parse::<f64>()
        .ok()
}

fn format_si_scaled(value: f64, steps: &[(&str, f64)]) -> String {
    // `steps` must be ordered from largest scale to smallest (GHz→Hz, s→ps).
    let abs = value.abs();
    let mut chosen = steps[steps.len() - 1];
    for &step in steps {
        if abs >= step.1 {
            chosen = step;
            break;
        }
    }
    let scaled = value / chosen.1;
    format!("{} {}", format_plain_number(scaled), chosen.0)
}

fn format_plain_number(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    let abs = value.abs();
    let text = if abs == 0.0 {
        "0".to_owned()
    } else if abs >= 100.0 {
        format!("{value:.2}")
    } else if abs >= 10.0 {
        format!("{value:.3}")
    } else if abs >= 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.4}")
    };
    trim_trailing_zeros(&text)
}

fn trim_trailing_zeros(text: &str) -> String {
    if !text.contains('.') {
        return text.to_owned();
    }
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" || trimmed == "+" {
        "0".into()
    } else {
        trimmed.to_owned()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    ScopePosition {
        channel: u8,
        divisions: f64,
    },
    ScopeMeasure {
        channel: u8,
        meas_type: ScopeMeasType,
    },
    ScopeTimebase(f64),
    ScopeTrigger {
        source: String,
        level: f64,
        slope: String,
    },
    SourceVoltage {
        channel: u8,
        value: f64,
    },
    SourceCurrent {
        channel: u8,
        value: f64,
    },
    SourceOvp {
        channel: u8,
        value: f64,
    },
    SourceOcp {
        channel: u8,
        value: f64,
    },
    SourceOutput {
        channel: u8,
        enabled: bool,
    },
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

    /// Open a soft demo session for UI debugging (no VISA / hardware required).
    pub fn connect_demo(kind: InstrumentKind) -> Result<Self, InstrumentError> {
        use super::DemoTransport;
        let resource = format!("DEMO::{}", kind.label());
        Self::from_transport(resource, Box::new(DemoTransport::for_kind(kind)), Some(kind))
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
            ScopeRun => self.scope_run(),
            ScopeStop => self.scope_stop(),
            ScopeSingle => self.scope_single(),
            ScopeAutoset => self.scope_autoset(),
            ScopeChannel { channel, enabled } => self.scope_select_channel(channel, enabled),
            ScopeScale {
                channel,
                volts_per_div,
            } => self.scope_set_scale(channel, volts_per_div),
            ScopeOffset { channel, volts } => self.scope_set_offset(channel, volts),
            ScopePosition {
                channel,
                divisions,
            } => self.scope_set_position(channel, divisions),
            ScopeMeasure {
                channel,
                meas_type,
            } => self.scope_measure(channel, meas_type),
            ScopeTimebase(seconds) => self.scope_set_timebase(seconds),
            ScopeTrigger {
                source,
                level,
                slope,
            } => self.scope_set_trigger(&source, level, &slope),
            SourceVoltage { channel, value } => {
                validate_range(value, self.profile.voltage_limit, "V")?;
                self.source_set_voltage(channel, value)
            }
            SourceCurrent { channel, value } => {
                validate_range(value, self.profile.current_limit, "A")?;
                self.source_set_current(channel, value)
            }
            SourceOvp { channel, value } => {
                validate_range(value, self.profile.voltage_limit, "V")?;
                self.source_set_ovp(channel, value)
            }
            SourceOcp { channel, value } => {
                validate_range(value, self.profile.current_limit, "A")?;
                self.source_set_ocp(channel, value)
            }
            SourceOutput { channel, enabled } => self.source_set_output(channel, enabled),
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
            InstrumentKind::DcSource => self.read_source_measurements(),
            InstrumentKind::ElectronicLoad => {
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
        // Screen hardcopy (PNG pixels), not CURVe numerical samples.
        if self.vendor_is("TEKTRONIX") {
            let _ = self.session.write("HEADer OFF");
            let _ = self.session.write("SAVe:IMAGe:FILEFormat PNG");
            let _ = self.session.write("SAVe:IMAGe:INKSaver ON");
            self.session.write("HARDCopy STARt")?;
            // HARDCopy may return an IEEE488.2 definite-length block; strip header if present.
            Ok(parse_ieee_block(&self.session.read_raw()?).to_vec())
        } else if self.vendor_is("RIGOL") || self.vendor_is("SIGLENT") {
            self.session.write(":DISPLAY:DATA? ON,OFF,PNG")?;
            Ok(parse_ieee_block(&self.session.read_raw()?).to_vec())
        } else {
            Err(InstrumentError::Unsupported(format!(
                "{} screen capture",
                self.profile.name
            )))
        }
    }

    /// Read the current **on-screen** waveform source over VISA (full displayed record).
    ///
    /// Priority by vendor:
    /// - **Tektronix**: native `.isf` / `.wfm` / `.csv` via filesystem, else `CURVe`→ISF
    /// - **Rigol/Siglent**: native `.wfm` via MMEM when available, else `:WAV NORM`→CSV
    /// - **Keysight/Agilent**: `:WAVeform` BYTE screen → host CSV
    /// - **Other**: try Keysight → Rigol/Siglent → Tek curve
    pub fn capture_scope_waveform_source(
        &mut self,
        channel: u8,
    ) -> Result<(Vec<u8>, String), InstrumentError> {
        self.require(InstrumentKind::Oscilloscope, "waveform source")?;
        let channel = channel.clamp(1, 8);
        if self.vendor_is("TEKTRONIX") {
            if let Ok(bytes) = self.read_tek_waveform_via_filesystem(channel, "isf") {
                return Ok((bytes, format!("waveform_CH{channel}.isf")));
            }
            if let Ok(bytes) = self.read_tek_waveform_via_filesystem(channel, "wfm") {
                return Ok((bytes, format!("waveform_CH{channel}.wfm")));
            }
            if let Ok(bytes) = self.read_tek_waveform_via_filesystem(channel, "csv") {
                return Ok((bytes, format!("waveform_CH{channel}.csv")));
            }
            let bytes = self.read_tek_isf_via_curve(channel)?;
            Ok((bytes, format!("waveform_CH{channel}.isf")))
        } else if self.is_rigol_siglent_family() {
            // Prefer :WAV:DATA→CSV first (reliable). Native .wfm via MMEM is optional and
            // can hang on models that ignore SAVE/*OPC — try it only after CSV fails.
            if let Ok(bytes) = self.export_screen_csv(channel) {
                return Ok((bytes, format!("waveform_CH{channel}.csv")));
            }
            if let Ok(bytes) = self.read_rigol_csv_via_filesystem(channel) {
                return Ok((bytes, format!("waveform_CH{channel}.csv")));
            }
            if let Ok(bytes) = self.read_rigol_wfm_via_filesystem(channel) {
                return Ok((bytes, format!("waveform_CH{channel}.wfm")));
            }
            Err(InstrumentError::Unsupported(
                "Rigol/Siglent screen waveform export failed".into(),
            ))
        } else if self.is_keysight_family() {
            let bytes = self.export_screen_csv(channel)?;
            Ok((bytes, format!("waveform_CH{channel}.csv")))
        } else {
            // Unknown brand: probe common SCPI families.
            if let Ok(bytes) = self.export_screen_csv(channel) {
                return Ok((bytes, format!("waveform_CH{channel}.csv")));
            }
            if let Ok(bytes) = self.read_tek_isf_via_curve(channel) {
                return Ok((bytes, format!("waveform_CH{channel}.isf")));
            }
            Err(InstrumentError::Unsupported(
                "unsupported oscilloscope waveform source protocol".into(),
            ))
        }
    }

    /// Build a spreadsheet CSV (`TIME,CHx`) from the on-screen waveform record.
    fn export_screen_csv(&mut self, channel: u8) -> Result<Vec<u8>, InstrumentError> {
        let trace = self.read_scope_screen_waveform(channel)?;
        if trace.x.is_empty() {
            return Err(InstrumentError::Unsupported(
                "empty on-screen waveform".into(),
            ));
        }
        Ok(crate::waveform_file::waveform_to_spreadsheet_csv(&trace))
    }

    fn is_keysight_family(&self) -> bool {
        self.vendor_is("KEYSIGHT") || self.vendor_is("AGILENT")
    }

    fn is_rigol_siglent_family(&self) -> bool {
        self.vendor_is("RIGOL") || self.vendor_is("SIGLENT")
    }

    /// Points in the current on-screen waveform for `channel`.
    fn scope_screen_point_count(&mut self, channel: u8) -> Result<usize, InstrumentError> {
        let channel = channel.clamp(1, 8);
        if self.is_rigol_siglent_family() {
            self.session
                .write(&format!(":WAV:SOUR CHAN{channel}"))?;
            self.session.write(":WAV:MODE NORM")?;
            self.session.write(":WAV:FORM BYTE")?;
            let count = self
                .session
                .query_f64(":WAV:POIN?")
                .unwrap_or(1000.0)
                .round() as usize;
            return Ok(count.max(100).min(10_000_000));
        }
        if self.is_keysight_family() {
            return self.keysight_screen_point_count(channel);
        }
        if self.vendor_is("TEKTRONIX") {
            return self.tek_screen_point_count(channel);
        }
        // Unknown: try Keysight then Rigol then Tek.
        if let Ok(n) = self.keysight_screen_point_count(channel) {
            return Ok(n);
        }
        self.session
            .write(&format!(":WAV:SOUR CHAN{channel}"))
            .ok();
        let _ = self.session.write(":WAV:MODE NORM");
        let _ = self.session.write(":WAV:FORM BYTE");
        if let Ok(n) = self.session.query_f64(":WAV:POIN?") {
            let n = n.round() as usize;
            if n >= 100 {
                return Ok(n.min(10_000_000));
            }
        }
        self.tek_screen_point_count(channel)
    }

    fn tek_screen_point_count(&mut self, channel: u8) -> Result<usize, InstrumentError> {
        self.session
            .write(&format!("DATa:SOUrce CH{channel}"))?;
        let _ = self.session.write("DATa:MODE SCREEN");
        let nr = self
            .session
            .query_f64("WFMOutpre:NR_Pt?")
            .unwrap_or(0.0)
            .round() as usize;
        if nr >= 100 {
            Ok(nr.min(10_000_000))
        } else {
            let hor = self
                .session
                .query_f64("HORizontal:RECOrdlength?")
                .unwrap_or(1000.0)
                .round() as usize;
            Ok(hor.max(100).min(10_000_000))
        }
    }

    fn keysight_screen_point_count(&mut self, channel: u8) -> Result<usize, InstrumentError> {
        self.prepare_keysight_screen_waveform(channel)?;
        if let Ok(pre) = self.session.query(":WAVeform:PREamble?") {
            if let Some(n) = parse_keysight_preamble_points(&pre) {
                return Ok(n.max(100).min(10_000_000));
            }
        }
        let n = self
            .session
            .query_f64(":WAVeform:POINts?")
            .unwrap_or(1000.0)
            .round() as usize;
        Ok(n.max(100).min(10_000_000))
    }

    fn prepare_keysight_screen_waveform(&mut self, channel: u8) -> Result<(), InstrumentError> {
        let channel = channel.clamp(1, 8);
        self.session
            .write(&format!(":WAVeform:SOURce CHANnel{channel}"))?;
        // NORMal = displayed (screen) record on InfiniiVision.
        let _ = self.session.write(":WAVeform:POINts:MODE NORMal");
        self.session.write(":WAVeform:FORMat BYTE")?;
        let _ = self.session.write(":WAVeform:UNSigned 1");
        let _ = self.session.write(":WAVeform:BYTeorder MSBFirst");
        Ok(())
    }

    /// Full on-screen waveform as [`WaveformTrace`] (no display downsampling).
    pub fn read_scope_screen_waveform(
        &mut self,
        channel: u8,
    ) -> Result<WaveformTrace, InstrumentError> {
        self.require(InstrumentKind::Oscilloscope, "waveform")?;
        let channel = channel.clamp(1, 8);
        if self.vendor_is("TEKTRONIX") {
            return self.read_tek_screen_waveform(channel);
        }
        if self.is_keysight_family() {
            return self.read_keysight_screen_waveform(channel);
        }
        if self.is_rigol_siglent_family() {
            return self.read_rigol_screen_waveform(channel);
        }
        // Unknown brand: try Keysight → Rigol/Siglent → Tek.
        if let Ok(t) = self.read_keysight_screen_waveform(channel) {
            if !t.x.is_empty() {
                return Ok(t);
            }
        }
        if let Ok(t) = self.read_rigol_screen_waveform(channel) {
            if !t.x.is_empty() {
                return Ok(t);
            }
        }
        self.read_tek_screen_waveform(channel)
    }

    fn read_rigol_screen_waveform(
        &mut self,
        channel: u8,
    ) -> Result<WaveformTrace, InstrumentError> {
        let points = {
            self.session
                .write(&format!(":WAV:SOUR CHAN{channel}"))?;
            self.session.write(":WAV:MODE NORM")?;
            self.session.write(":WAV:FORM BYTE")?;
            self.session
                .query_f64(":WAV:POIN?")
                .unwrap_or(1000.0)
                .round()
                .max(100.0)
                .min(10_000_000.0) as usize
        };
        let _ = self.session.write(":WAV:STAR 1");
        let _ = self.session.write(&format!(":WAV:STOP {points}"));
        let _ = self.session.write(&format!(":WAV:POIN {points}"));
        let xincr = self.session.query_f64(":WAV:XINC?")?;
        let xzero = self.session.query_f64(":WAV:XOR?").unwrap_or(0.0);
        let ymult = self.session.query_f64(":WAV:YINC?")?;
        let yoff = self.session.query_f64(":WAV:YOR?").unwrap_or(0.0);
        let yzero = self.session.query_f64(":WAV:YREF?").unwrap_or(0.0);
        let y_scale = if self.vendor_is("SIGLENT") {
            YScaleKind::Siglent
        } else {
            YScaleKind::Rigol
        };
        let y_unit = self.query_scope_y_unit(channel);
        self.session.write(":WAV:DATA?")?;
        let raw = self.session.read_raw()?;
        let data = parse_ieee_block(&raw);
        decode_scope_bytes(
            data,
            channel,
            xincr,
            xzero,
            0.0,
            ymult,
            yoff,
            yzero,
            false,
            y_scale,
            "s",
            &y_unit,
        )
    }

    fn read_keysight_screen_waveform(
        &mut self,
        channel: u8,
    ) -> Result<WaveformTrace, InstrumentError> {
        self.prepare_keysight_screen_waveform(channel)?;
        let (points, xincr, xzero, xref, ymult, yoff, yzero) =
            if let Ok(pre) = self.session.query(":WAVeform:PREamble?") {
                if let Some(p) = parse_keysight_preamble(&pre) {
                    p
                } else {
                    self.keysight_scale_queries()?
                }
            } else {
                self.keysight_scale_queries()?
            };
        let y_unit = self.query_scope_y_unit(channel);
        let _ = self.session.write(&format!(":WAVeform:POINts {points}"));
        self.session.write(":WAVeform:DATA?")?;
        let raw = self.session.read_raw()?;
        let data = parse_ieee_block(&raw);
        decode_scope_bytes(
            data,
            channel,
            xincr,
            xzero,
            xref,
            ymult,
            yoff,
            yzero,
            false,
            YScaleKind::Keysight,
            "s",
            &y_unit,
        )
    }

    fn keysight_scale_queries(
        &mut self,
    ) -> Result<(usize, f64, f64, f64, f64, f64, f64), InstrumentError> {
        let points = self
            .session
            .query_f64(":WAVeform:POINts?")
            .unwrap_or(1000.0)
            .round()
            .max(100.0)
            .min(10_000_000.0) as usize;
        Ok((
            points,
            self.session.query_f64(":WAVeform:XINCrement?")?,
            self.session.query_f64(":WAVeform:XORigin?").unwrap_or(0.0),
            self.session
                .query_f64(":WAVeform:XREFerence?")
                .unwrap_or(0.0),
            self.session.query_f64(":WAVeform:YINCrement?")?,
            self.session.query_f64(":WAVeform:YORigin?").unwrap_or(0.0),
            self.session
                .query_f64(":WAVeform:YREFerence?")
                .unwrap_or(0.0),
        ))
    }

    fn read_tek_screen_waveform(
        &mut self,
        channel: u8,
    ) -> Result<WaveformTrace, InstrumentError> {
        let points = self.tek_screen_point_count(channel)?;
        self.session
            .write(&format!("DATa:SOUrce CH{channel}"))?;
        self.session.write("DATa:ENCdg RIBINARY")?;
        self.session.write("DATa:WIDth 1")?;
        let _ = self.session.write("DATa:MODE SCREEN");
        self.session.write("DATa:STARt 1")?;
        self.session.write(&format!("DATa:STOP {points}"))?;
        let xincr = self.session.query_f64("WFMOutpre:XINcr?")?;
        let xzero = self.session.query_f64("WFMOutpre:XZEro?")?;
        let ymult = self.session.query_f64("WFMOutpre:YMUlt?")?;
        let yoff = self.session.query_f64("WFMOutpre:YOFf?")?;
        let yzero = self.session.query_f64("WFMOutpre:YZEro?")?;
        let pt_off = self.session.query_f64("WFMOutpre:PT_Off?").unwrap_or(0.0);
        let y_unit = self.query_scope_y_unit(channel);
        self.session.write("CURVe?")?;
        let raw = self.session.read_raw()?;
        let data = parse_ieee_block(&raw);
        decode_scope_bytes(
            data,
            channel,
            xincr,
            xzero,
            pt_off,
            ymult,
            yoff,
            yzero,
            true,
            YScaleKind::Tek,
            "s",
            &y_unit,
        )
    }

    /// Query vertical unit (supports voltage / current probes). Falls back to `"V"`.
    fn query_scope_y_unit(&mut self, channel: u8) -> String {
        let channel = channel.clamp(1, 8);
        let candidates: &[&str] = if self.vendor_is("TEKTRONIX") {
            &[
                "WFMOutpre:YUNit?",
                "WFMOutpre:YUNIT?",
                "WFMOutpre:YUNits?",
            ]
        } else if self.is_keysight_family() {
            &[
                ":WAVeform:YUNits?",
                ":WAVeform:YUNIT?",
                ":WAVeform:YUNit?",
            ]
        } else {
            // Rigol / Siglent channel units (current probe → A).
            &[]
        };
        for cmd in candidates {
            if let Ok(raw) = self.session.query(cmd) {
                let u = normalize_wave_unit(&raw, "");
                if !u.is_empty() {
                    return u;
                }
            }
        }
        if self.is_rigol_siglent_family() {
            for cmd in [
                format!(":CHANnel{channel}:UNITs?"),
                format!(":CHAN{channel}:UNIT?"),
                format!(":CHANnel{channel}:UNIT?"),
            ] {
                if let Ok(raw) = self.session.query(&cmd) {
                    let u = normalize_wave_unit(&raw, "");
                    if !u.is_empty() {
                        return u;
                    }
                }
            }
        }
        "V".into()
    }

    /// Tektronix: save waveform to scope disk, then `FILESystem:READFile`.
    fn read_tek_waveform_via_filesystem(
        &mut self,
        channel: u8,
        kind: &str,
    ) -> Result<Vec<u8>, InstrumentError> {
        self.prepare_tek_screen_waveform(channel)?;
        let (file_format, ext) = match kind {
            "wfm" => ("WINDows", "wfm"),
            "csv" => ("SPREADSheet", "csv"),
            _ => ("INTERNal", "isf"),
        };
        // Prefer C: then E: (USB) — some benches only allow USB mass storage.
        for path in [
            format!("C:/WiParse_tmp.{ext}"),
            format!("E:/WiParse_tmp.{ext}"),
        ] {
            let _ = self.session.write("HEADer OFF");
            if self
                .session
                .write(&format!("SAVe:WAVEform:FILEFormat {file_format}"))
                .is_err()
            {
                continue;
            }
            if self
                .session
                .write(&format!("SAVe:WAVEform CH{channel},\"{path}\""))
                .is_err()
            {
                continue;
            }
            let _ = self.session.query("*OPC?");
            if self
                .session
                .write(&format!("FILESystem:READFile \"{path}\""))
                .is_err()
            {
                continue;
            }
            let Ok(raw) = self.session.read_raw() else {
                continue;
            };
            let _ = self.session.write(&format!("FILESystem:DELEte \"{path}\""));
            let data = parse_ieee_block(&raw).to_vec();
            if tek_waveform_source_bytes_ok(&data, ext) {
                return Ok(data);
            }
        }
        Err(InstrumentError::Unsupported(format!(
            "Tektronix FILESystem:READFile returned empty/invalid .{ext}"
        )))
    }

    /// Select channel and gate Tektronix save/transfer to the on-screen waveform record.
    fn prepare_tek_screen_waveform(&mut self, channel: u8) -> Result<(), InstrumentError> {
        let channel = channel.clamp(1, 8);
        let _ = self.session.write("HEADer OFF");
        self.session
            .write(&format!("DATa:SOUrce CH{channel}"))?;
        let _ = self.session.write("DATa:MODE SCREEN");
        // MDO/MSO/DPO: limit SAVe:WAVEform to visible screen (not full memory).
        let _ = self.session.write("SAVe:WAVEform:GATIng SCREEN");
        Ok(())
    }

    /// Rigol/Siglent: optional CSV via `:SAVE:CSV` / `:SAVE:WAVeform` then `:MMEM:DATA?`.
    ///
    /// Does **not** use `:MMEM:LOAD:TRACe` (that loads a file *into* the scope).
    fn read_rigol_csv_via_filesystem(&mut self, channel: u8) -> Result<Vec<u8>, InstrumentError> {
        let _ = self.session.write(&format!(":WAV:SOUR CHAN{channel}"));
        let _ = self.session.write(":WAV:MODE NORM");
        let _ = self.session.write(":SAVE:CSV:LENGth DISPlay");
        let _ = self
            .session
            .write(&format!(":SAVE:CSV:CHANnel CHAN{channel},ON"));

        let attempts = [
            (
                format!(":SAVE:CSV \"/wiparse_ch{channel}.csv\""),
                format!("/wiparse_ch{channel}.csv"),
            ),
            (
                format!(":SAVE:CSV \"C:/wiparse_ch{channel}.csv\""),
                format!("C:/wiparse_ch{channel}.csv"),
            ),
            (
                format!(":SAVE:WAVeform \"/wiparse_ch{channel}.csv\""),
                format!("/wiparse_ch{channel}.csv"),
            ),
        ];
        for (save_cmd, path) in attempts {
            let _ = self.session.write(&save_cmd);
            let _ = self.session.query("*OPC?");
            if self
                .session
                .write(&format!(":MMEM:DATA? \"{path}\""))
                .is_err()
            {
                continue;
            }
            let Ok(raw) = self.session.read_raw() else {
                continue;
            };
            let data = parse_ieee_block(&raw).to_vec();
            if data.len() >= 16 && looks_like_text_csv(&data) {
                let _ = self.session.write(&format!(":MMEM:DELEte \"{path}\""));
                let _ = self.session.write(&format!(":MMEM:DEL \"{path}\""));
                return Ok(data);
            }
        }
        Err(InstrumentError::Unsupported(
            "Rigol/Siglent MMEM CSV export unavailable".into(),
        ))
    }

    /// Rigol/Siglent: try saving a proprietary `.wfm` to scope storage and read it back.
    ///
    /// Not all models expose this over SCPI; callers should fall back to `:WAV:DATA` CSV.
    fn read_rigol_wfm_via_filesystem(&mut self, channel: u8) -> Result<Vec<u8>, InstrumentError> {
        let _ = self.session.write(&format!(":WAV:SOUR CHAN{channel}"));
        let _ = self.session.write(":WAV:MODE NORM");
        // Format hints used by various Rigol generations (ignored when unsupported).
        for fmt in [
            ":SAVE:WAVeform:TYPE WFM",
            ":SAVE:WAVeform:FORMat WFM",
            ":STORage:WAVeform:FORMat WFM",
        ] {
            let _ = self.session.write(fmt);
        }
        let attempts = [
            format!(":SAVE:WAVeform \"/wiparse_ch{channel}.wfm\""),
            format!(":SAVE:WAVeform \"C:/wiparse_ch{channel}.wfm\""),
            format!(":SAVE:WAVeform CHAN{channel},\"/wiparse_ch{channel}.wfm\""),
            format!(":MMEM:STOR:WAV \"/wiparse_ch{channel}.wfm\""),
            format!(":STORage:WAVeform \"/wiparse_ch{channel}.wfm\""),
        ];
        let paths = [
            format!("/wiparse_ch{channel}.wfm"),
            format!("C:/wiparse_ch{channel}.wfm"),
        ];
        for save_cmd in attempts {
            // Do not wait on *OPC? — unsupported SAVE formats can stall the bus.
            if self.session.write(&save_cmd).is_err() {
                continue;
            }
            for path in &paths {
                if self
                    .session
                    .write(&format!(":MMEM:DATA? \"{path}\""))
                    .is_err()
                {
                    continue;
                }
                let Ok(raw) = self.session.read_raw() else {
                    continue;
                };
                let data = parse_ieee_block(&raw).to_vec();
                if crate::rigol_wfm::looks_like_rigol_wfm(&data)
                    && crate::waveform_file::load_waveform_bytes(
                        &data,
                        "wfm",
                        &format!("CH{channel}"),
                    )
                    .is_ok()
                {
                    let _ = self.session.write(&format!(":MMEM:DELEte \"{path}\""));
                    let _ = self.session.write(&format!(":MMEM:DEL \"{path}\""));
                    return Ok(data);
                }
            }
        }
        Err(InstrumentError::Unsupported(
            "Rigol/Siglent native WFM export unavailable".into(),
        ))
    }

    /// Tektronix: `SAVe:WAVEform` INTERNAL to scope disk, then `FILESystem:READFile`.
    fn read_tek_isf_via_filesystem(&mut self, channel: u8) -> Result<Vec<u8>, InstrumentError> {
        self.read_tek_waveform_via_filesystem(channel, "isf")
    }

    /// Tektronix: assemble an ISF-compatible blob from on-screen `WFMOutpre?` + `CURVe?`.
    fn read_tek_isf_via_curve(&mut self, channel: u8) -> Result<Vec<u8>, InstrumentError> {
        self.prepare_tek_screen_waveform(channel)?;
        let _ = self.session.write("HEADer ON");
        self.session.write("DATa:ENCdg RIBINARY")?;
        self.session.write("DATa:WIDth 1")?;
        let points = self.scope_screen_point_count(channel)?;
        self.session.write("DATa:STARt 1")?;
        self.session.write(&format!("DATa:STOP {points}"))?;
        let preamble = self.session.query("WFMOutpre?")?;
        self.session.write("CURVe?")?;
        let curve = self.session.read_raw()?;
        Ok(assemble_tek_isf(&preamble, &curve))
    }

    pub fn read_scope_waveform(
        &mut self,
        channel: u8,
        max_points: usize,
    ) -> Result<WaveformTrace, InstrumentError> {
        self.require(InstrumentKind::Oscilloscope, "waveform")?;
        let channel = channel.clamp(1, 8);
        let max_points = max_points.clamp(100, 1_000_000);
        // Prefer the same vendor-aware screen path as “Read Wave Source”, then downsample.
        let mut trace = self.read_scope_screen_waveform(channel)?;
        if trace.x.len() > max_points {
            let (x, y) = downsample_minmax(&trace.x, &trace.y, max_points);
            trace.x = x;
            trace.y = y;
        }
        Ok(trace)
    }

    fn vendor_is(&self, needle: &str) -> bool {
        self.identity.manufacturer.to_uppercase().contains(needle)
    }

    fn source_channel_count(&self) -> u8 {
        self.profile.capabilities.channels.max(1).min(8)
    }

    fn source_uses_sour_prefix(&self) -> bool {
        self.source_channel_count() > 1
            && (self.vendor_is("RIGOL")
                || self.vendor_is("SIGLENT")
                || self.vendor_is("ITECH")
                || self.identity.model.to_uppercase().starts_with("DP"))
    }

    fn source_set_voltage(
        &mut self,
        channel: u8,
        value: f64,
    ) -> Result<Option<String>, InstrumentError> {
        let channels = self.source_channel_count();
        let ch = channel.clamp(1, channels);
        let uses_sour = self.source_uses_sour_prefix();
        let session = self.require(InstrumentKind::DcSource, "set voltage")?;
        if uses_sour {
            session
                .write(&format!(":SOUR{ch}:VOLT {value}"))
                .map(|_| None)
        } else if channels > 1 {
            session.write(&format!("INST:NSEL {ch}"))?;
            session.write(&format!("VOLT {value}")).map(|_| None)
        } else {
            session.write(&format!("VOLT {value}")).map(|_| None)
        }
    }

    fn source_set_current(
        &mut self,
        channel: u8,
        value: f64,
    ) -> Result<Option<String>, InstrumentError> {
        let channels = self.source_channel_count();
        let ch = channel.clamp(1, channels);
        let uses_sour = self.source_uses_sour_prefix();
        let session = self.require(InstrumentKind::DcSource, "set current")?;
        if uses_sour {
            session
                .write(&format!(":SOUR{ch}:CURR {value}"))
                .map(|_| None)
        } else if channels > 1 {
            session.write(&format!("INST:NSEL {ch}"))?;
            session.write(&format!("CURR {value}")).map(|_| None)
        } else {
            session.write(&format!("CURR {value}")).map(|_| None)
        }
    }

    fn source_set_ovp(
        &mut self,
        channel: u8,
        value: f64,
    ) -> Result<Option<String>, InstrumentError> {
        let channels = self.source_channel_count();
        let ch = channel.clamp(1, channels);
        let uses_sour = self.source_uses_sour_prefix();
        let session = self.require(InstrumentKind::DcSource, "OVP")?;
        if uses_sour {
            session
                .write(&format!(":SOUR{ch}:VOLT:PROT {value}"))
                .map(|_| None)
        } else if channels > 1 {
            session.write(&format!("INST:NSEL {ch}"))?;
            session.write(&format!("VOLT:PROT {value}")).map(|_| None)
        } else {
            session.write(&format!("VOLT:PROT {value}")).map(|_| None)
        }
    }

    fn source_set_ocp(
        &mut self,
        channel: u8,
        value: f64,
    ) -> Result<Option<String>, InstrumentError> {
        let channels = self.source_channel_count();
        let ch = channel.clamp(1, channels);
        let uses_sour = self.source_uses_sour_prefix();
        let session = self.require(InstrumentKind::DcSource, "OCP")?;
        if uses_sour {
            session
                .write(&format!(":SOUR{ch}:CURR:PROT {value}"))
                .map(|_| None)
        } else if channels > 1 {
            session.write(&format!("INST:NSEL {ch}"))?;
            session.write(&format!("CURR:PROT {value}")).map(|_| None)
        } else {
            session.write(&format!("CURR:PROT {value}")).map(|_| None)
        }
    }

    fn source_set_output(
        &mut self,
        channel: u8,
        enabled: bool,
    ) -> Result<Option<String>, InstrumentError> {
        let channels = self.source_channel_count();
        let ch = channel.clamp(1, channels);
        let uses_sour = self.source_uses_sour_prefix();
        let state = on_off(enabled);
        let session = self.require(InstrumentKind::DcSource, "output")?;
        if uses_sour {
            // Rigol DP800 / similar: OUTP CH1,ON
            session
                .write(&format!(":OUTP CH{ch},{state}"))
                .map(|_| None)
        } else if channels > 1 {
            session.write(&format!("INST:NSEL {ch}"))?;
            session.write(&format!("OUTP {state}")).map(|_| None)
        } else {
            session.write(&format!("OUTP {state}")).map(|_| None)
        }
    }

    fn read_source_measurements(&mut self) -> Result<Vec<Reading>, InstrumentError> {
        let n = self.source_channel_count();
        let mut out = Vec::with_capacity((n as usize) * 3);
        if n <= 1 {
            let voltage = self.session.query_f64("MEAS:VOLT?")?;
            let current = self.session.query_f64("MEAS:CURR?")?;
            out.push(Reading {
                channel: "CH1 Voltage".into(),
                value: voltage,
                unit: "V".into(),
            });
            out.push(Reading {
                channel: "CH1 Current".into(),
                value: current,
                unit: "A".into(),
            });
            out.push(Reading {
                channel: "CH1 Power".into(),
                value: voltage * current,
                unit: "W".into(),
            });
            return Ok(out);
        }
        for ch in 1..=n {
            let voltage = self
                .session
                .query_f64(&format!("MEAS:VOLT? CH{ch}"))
                .or_else(|_| {
                    self.session.write(&format!("INST:NSEL {ch}"))?;
                    self.session.query_f64("MEAS:VOLT?")
                })?;
            let current = self
                .session
                .query_f64(&format!("MEAS:CURR? CH{ch}"))
                .or_else(|_| {
                    self.session.write(&format!("INST:NSEL {ch}"))?;
                    self.session.query_f64("MEAS:CURR?")
                })?;
            out.push(Reading {
                channel: format!("CH{ch} Voltage"),
                value: voltage,
                unit: "V".into(),
            });
            out.push(Reading {
                channel: format!("CH{ch} Current"),
                value: current,
                unit: "A".into(),
            });
            out.push(Reading {
                channel: format!("CH{ch} Power"),
                value: voltage * current,
                unit: "W".into(),
            });
        }
        Ok(out)
    }

    fn scope_run(&mut self) -> Result<Option<String>, InstrumentError> {
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "run")?;
        if tek {
            session.write("ACQuire:STOPAfter RUNSTop")?;
            session.write("ACQuire:STATE RUN").map(|_| None)
        } else if asian {
            session.write(":RUN").map(|_| None)
        } else {
            session.write("RUN").map(|_| None)
        }
    }

    fn scope_stop(&mut self) -> Result<Option<String>, InstrumentError> {
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "stop")?;
        if tek {
            session.write("ACQuire:STATE STOP").map(|_| None)
        } else if asian {
            session.write(":STOP").map(|_| None)
        } else {
            session.write("STOP").map(|_| None)
        }
    }

    fn scope_single(&mut self) -> Result<Option<String>, InstrumentError> {
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "single")?;
        if tek {
            session.write("ACQuire:STOPAfter SEQuence")?;
            session.write("ACQuire:STATE RUN").map(|_| None)
        } else if asian {
            session.write(":SINGle").map(|_| None)
        } else {
            session.write("SINGLE").map(|_| None)
        }
    }

    fn scope_autoset(&mut self) -> Result<Option<String>, InstrumentError> {
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "autoset")?;
        if tek {
            session.write("AUTOSet EXECute").map(|_| None)
        } else if asian {
            session.write(":AUToscale").map(|_| None)
        } else {
            session.write("AUTOSET EXECUTE").map(|_| None)
        }
    }

    fn scope_select_channel(
        &mut self,
        channel: u8,
        enabled: bool,
    ) -> Result<Option<String>, InstrumentError> {
        let channel = channel.clamp(1, 8);
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "channel")?;
        if tek {
            session
                .write(&format!("SELect:CH{channel} {}", on_off(enabled)))
                .map(|_| None)
        } else if asian {
            session
                .write(&format!(":CHANnel{channel}:DISPlay {}", on_off(enabled)))
                .map(|_| None)
        } else {
            session
                .write(&format!("SELECT:CH{channel} {}", on_off(enabled)))
                .map(|_| None)
        }
    }

    fn scope_set_scale(
        &mut self,
        channel: u8,
        volts_per_div: f64,
    ) -> Result<Option<String>, InstrumentError> {
        validate(volts_per_div, 1e-6, 1_000.0, "V/div")?;
        let channel = channel.clamp(1, 8);
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "vertical scale")?;
        if tek {
            session
                .write(&format!("CH{channel}:SCAle {volts_per_div}"))
                .map(|_| None)
        } else if asian {
            session
                .write(&format!(":CHANnel{channel}:SCALe {volts_per_div}"))
                .map(|_| None)
        } else {
            session
                .write(&format!("CH{channel}:SCALE {volts_per_div}"))
                .map(|_| None)
        }
    }

    fn scope_set_offset(
        &mut self,
        channel: u8,
        volts: f64,
    ) -> Result<Option<String>, InstrumentError> {
        validate(volts, -1_000.0, 1_000.0, "V")?;
        let channel = channel.clamp(1, 8);
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "vertical offset")?;
        if tek {
            session
                .write(&format!("CH{channel}:OFFSet {volts}"))
                .map(|_| None)
        } else if asian {
            session
                .write(&format!(":CHANnel{channel}:OFFSet {volts}"))
                .map(|_| None)
        } else {
            session
                .write(&format!("CH{channel}:OFFSET {volts}"))
                .map(|_| None)
        }
    }

    fn scope_set_position(
        &mut self,
        channel: u8,
        divisions: f64,
    ) -> Result<Option<String>, InstrumentError> {
        validate(divisions, -10.0, 10.0, "div")?;
        let channel = channel.clamp(1, 8);
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "vertical position")?;
        if tek {
            session
                .write(&format!("CH{channel}:POSition {divisions}"))
                .map(|_| None)
        } else if asian {
            session
                .write(&format!(":CHANnel{channel}:OFFSet {divisions}"))
                .map(|_| None)
        } else {
            session
                .write(&format!("CH{channel}:POSITION {divisions}"))
                .map(|_| None)
        }
    }

    fn scope_measure(
        &mut self,
        channel: u8,
        meas_type: ScopeMeasType,
    ) -> Result<Option<String>, InstrumentError> {
        let channel = channel.clamp(1, 8);
        let typ = meas_type.scpi();
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "measure")?;
        if tek {
            session.write(&format!("MEASUrement:IMMed:SOUrce1 CH{channel}"))?;
            session.write(&format!("MEASUrement:IMMed:TYPe {typ}"))?;
            let raw = session.query("MEASUrement:IMMed:VALue?")?.trim().to_owned();
            let units = session
                .query("MEASUrement:IMMed:UNIts?")
                .unwrap_or_default()
                .trim()
                .to_owned();
            let reading = format_scope_measure_payload(&raw, meas_type, &units);
            Ok(Some(format!(
                "CH{channel} {}: {reading}",
                meas_type.label()
            )))
        } else if asian {
            session.write(&format!(":MEASure:ITEM {typ},CHANnel{channel}"))?;
            let raw = session
                .query(&format!(":MEASure:ITEM? {typ},CHANnel{channel}"))?
                .trim()
                .to_owned();
            let reading = format_scope_measure_payload(&raw, meas_type, "");
            Ok(Some(format!(
                "CH{channel} {}: {reading}",
                meas_type.label()
            )))
        } else {
            session.write(&format!("MEASUREMENT:IMMED:SOURCE1 CH{channel}"))?;
            session.write(&format!("MEASUREMENT:IMMED:TYPE {typ}"))?;
            let raw = session.query("MEASUREMENT:IMMED:VALUE?")?.trim().to_owned();
            let reading = format_scope_measure_payload(&raw, meas_type, "");
            Ok(Some(format!(
                "CH{channel} {}: {reading}",
                meas_type.label()
            )))
        }
    }

    fn scope_set_timebase(&mut self, seconds: f64) -> Result<Option<String>, InstrumentError> {
        validate(seconds, 1e-9, 1_000.0, "s/div")?;
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "timebase")?;
        if tek {
            session
                .write(&format!("HORizontal:SCAle {seconds}"))
                .map(|_| None)
        } else if asian {
            session
                .write(&format!(":TIMebase:SCALe {seconds}"))
                .map(|_| None)
        } else {
            session
                .write(&format!("HORIZONTAL:SCALE {seconds}"))
                .map(|_| None)
        }
    }

    fn scope_set_trigger(
        &mut self,
        source: &str,
        level: f64,
        slope: &str,
    ) -> Result<Option<String>, InstrumentError> {
        validate(level, -1_000.0, 1_000.0, "V")?;
        let source = source.trim().to_uppercase();
        let slope = slope.trim().to_uppercase();
        let tek = self.vendor_is("TEKTRONIX");
        let asian = self.vendor_is("RIGOL") || self.vendor_is("SIGLENT");
        let session = self.require(InstrumentKind::Oscilloscope, "trigger")?;
        if tek {
            session.write(&format!("TRIGger:A:EDGE:SOUrce {source}"))?;
            session.write(&format!("TRIGger:A:EDGE:SLOpe {slope}"))?;
            session
                .write(&format!("TRIGger:A:LEVel {level}"))
                .map(|_| None)
        } else if asian {
            session.write(&format!(":TRIGger:EDGE:SOURce {source}"))?;
            session.write(&format!(":TRIGger:EDGE:SLOPe {slope}"))?;
            session
                .write(&format!(":TRIGger:EDGE:LEVel {level}"))
                .map(|_| None)
        } else {
            session.write(&format!("TRIGGER:A:EDGE:SOURCE {source}"))?;
            session.write(&format!("TRIGGER:A:EDGE:SLOPE {slope}"))?;
            session
                .write(&format!("TRIGGER:A:LEVEL {level}"))
                .map(|_| None)
        }
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

#[derive(Clone, Copy)]
enum YScaleKind {
    Tek,
    Rigol,
    Siglent,
    /// Keysight/Agilent InfiniiVision: v = YORigin + (data - YREFerence) * YINCrement
    Keysight,
}

fn scale_scope_y(kind: YScaleKind, code: f64, ymult: f64, yoff: f64, yzero: f64) -> f64 {
    match kind {
        // Tek: y = YZERO + YMULT * (code - YOFF)
        YScaleKind::Tek => yzero + ymult * (code - yoff),
        // Rigol DS/MSO: v = (data - YORigin - YREFerence) * YINCrement
        YScaleKind::Rigol => (code - yoff - yzero) * ymult,
        // Siglent SDS family commonly: v = (data - YREF) * YINC + YOR
        YScaleKind::Siglent => (code - yzero) * ymult + yoff,
        // Keysight: v = YORigin + (data - YREFerence) * YINCrement
        // (ymult=YINC, yoff=YOR, yzero=YREF)
        YScaleKind::Keysight => yoff + (code - yzero) * ymult,
    }
}

fn decode_scope_bytes(
    data: &[u8],
    channel: u8,
    xincr: f64,
    xzero: f64,
    pt_off: f64,
    ymult: f64,
    yoff: f64,
    yzero: f64,
    signed: bool,
    y_scale: YScaleKind,
    x_unit: &str,
    y_unit: &str,
) -> Result<WaveformTrace, InstrumentError> {
    if data.is_empty() {
        return Err(InstrumentError::Unsupported("empty waveform block".into()));
    }
    let mut x = Vec::with_capacity(data.len());
    let mut y = Vec::with_capacity(data.len());
    for (index, byte) in data.iter().copied().enumerate() {
        let code = if signed {
            (byte as i8) as f64
        } else {
            byte as f64
        };
        x.push(xzero + (index as f64 - pt_off) * xincr);
        y.push(scale_scope_y(y_scale, code, ymult, yoff, yzero));
    }
    Ok(WaveformTrace {
        channel: format!("CH{channel}"),
        x,
        y,
        x_unit: normalize_wave_unit(x_unit, "s"),
        y_unit: normalize_wave_unit(y_unit, "V"),
    })
}

/// Strip quotes / NULs from instrument unit strings (`"A"`, `A\0…`, `Amps` → `A`).
fn normalize_wave_unit(raw: &str, default: &str) -> String {
    let s = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches('\0')
        .trim();
    if s.is_empty() {
        return default.to_string();
    }
    // Reject numeric / status replies from unsupported unit queries (e.g. demo `"0"`).
    if s.chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == '+' || c == '-' || c == 'e' || c == 'E')
    {
        return default.to_string();
    }
    match s.to_ascii_lowercase().as_str() {
        "a" | "aa" | "amp" | "amps" | "ampere" | "amperes" => "A".into(),
        "v" | "volt" | "volts" => "V".into(),
        "w" | "watt" | "watts" => "W".into(),
        "s" | "sec" | "second" | "seconds" => "s".into(),
        other => {
            // Keep short unit tokens (A, V, mA, …); drop long descriptive strings.
            if other.len() <= 8 && other.chars().any(|c| c.is_ascii_alphabetic()) {
                s.to_string()
            } else {
                default.to_string()
            }
        }
    }
}

/// Parse Keysight `:WAVeform:PREamble?` → (points, xinc, xorig, xref, yinc, yorig, yref).
fn parse_keysight_preamble(pre: &str) -> Option<(usize, f64, f64, f64, f64, f64, f64)> {
    let parts: Vec<&str> = pre.trim().split(',').map(str::trim).collect();
    if parts.len() < 10 {
        return None;
    }
    let points = parts[2].parse::<f64>().ok()?.round() as usize;
    let xinc: f64 = parts[4].parse().ok()?;
    let xorig: f64 = parts[5].parse().ok()?;
    let xref: f64 = parts[6].parse().ok()?;
    let yinc: f64 = parts[7].parse().ok()?;
    let yorig: f64 = parts[8].parse().ok()?;
    let yref: f64 = parts[9].parse().ok()?;
    if points < 2 || !xinc.is_finite() || !yinc.is_finite() {
        return None;
    }
    Some((points, xinc, xorig, xref, yinc, yorig, yref))
}

fn parse_keysight_preamble_points(pre: &str) -> Option<usize> {
    parse_keysight_preamble(pre).map(|(n, ..)| n)
}

fn looks_like_text_csv(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
    let upper = head.to_ascii_uppercase();
    head.contains(',')
        && (upper.contains("TIME")
            || upper.contains("CH")
            || upper.contains("CHANNEL")
            || upper.contains("SECOND")
            || upper.contains("VOLT"))
}

/// Accept filesystem waveform only when content matches the requested format and parses.
/// Ensures Tek `WINDows` `.wfm` (`WFM#001` / `#003`) is actually loadable before returning.
fn tek_waveform_source_bytes_ok(bytes: &[u8], ext: &str) -> bool {
    if bytes.len() < 32 {
        return false;
    }
    match ext {
        "wfm" => crate::waveform_file::load_waveform_bytes(bytes, "wfm", "CH1").is_ok(),
        "isf" => crate::waveform_file::load_waveform_bytes(bytes, "isf", "CH1").is_ok(),
        "csv" => looks_like_text_csv(bytes),
        _ => !bytes.is_empty(),
    }
}

/// Build an ISF-compatible byte stream from Tektronix bus preamble + CURVe block.
fn assemble_tek_isf(preamble: &str, curve_raw: &[u8]) -> Vec<u8> {
    let mut pre = preamble.trim().replace("WFMOUTPRE:", "WFMPRE:");
    pre = pre.replace("WFMOutpre:", "WFMPRE:");
    pre = pre.replace("wfmoutpre:", "WFMPRE:");
    if !pre.starts_with(':') && !pre.is_empty() {
        pre.insert(0, ':');
    }
    let mut out = Vec::with_capacity(pre.len() + curve_raw.len() + 16);
    out.extend_from_slice(pre.as_bytes());
    if !pre.ends_with(';') && !pre.ends_with('\n') {
        out.push(b';');
    }
    let curve = if curve_raw.windows(6).any(|w| w.eq_ignore_ascii_case(b":CURVE"))
        || curve_raw.windows(6).any(|w| w.eq_ignore_ascii_case(b"CURVE "))
    {
        curve_raw.to_vec()
    } else if curve_raw.first() == Some(&b'#') {
        let mut v = b":CURVE ".to_vec();
        v.extend_from_slice(curve_raw);
        v
    } else {
        // Wrap bare payload as IEEE488.2 definite-length block: #<ndigits><digits><data>
        let len = curve_raw.len().to_string();
        let mut v = b":CURVE #".to_vec();
        v.push(b'0' + len.len() as u8);
        v.extend_from_slice(len.as_bytes());
        v.extend_from_slice(curve_raw);
        v
    };
    out.extend_from_slice(&curve);
    out
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

        assert_eq!(
            classify_instrument_kind("RIGOL TECHNOLOGIES", "DS1054Z"),
            InstrumentKind::Oscilloscope
        );
        assert_eq!(
            classify_instrument_kind("RIGOL TECHNOLOGIES", "DP832"),
            InstrumentKind::DcSource
        );
        assert_eq!(
            classify_instrument_kind("Keysight Technologies", "34461A"),
            InstrumentKind::Multimeter
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
            device.execute(ControlCommand::SourceVoltage {
                channel: 1,
                value: 1000.0
            }),
            Err(InstrumentError::OutOfRange { .. })
        ));
    }

    #[test]
    fn estimates_common_dc_source_channels() {
        assert_eq!(estimate_dc_source_channels("DP832"), 3);
        assert_eq!(estimate_dc_source_channels("DP821A"), 2);
        assert_eq!(estimate_dc_source_channels("DP711"), 1);
        assert_eq!(estimate_dc_source_channels("E3631A"), 3);
        assert_eq!(estimate_dc_source_channels("DP999X"), 1);
    }

    #[test]
    fn demo_connect_opens_scope_and_source() {
        let mut scope = InstrumentDevice::connect_demo(InstrumentKind::Oscilloscope).unwrap();
        assert_eq!(scope.profile.kind, InstrumentKind::Oscilloscope);
        assert!(scope.profile.capabilities.screenshot);
        assert!(scope.profile.capabilities.waveform);
        let png = scope.capture_scope_png().unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let trace = scope.read_scope_screen_waveform(1).unwrap();
        assert!(trace.y.len() >= 64);
        let (src, name) = scope.capture_scope_waveform_source(1).unwrap();
        assert!(!src.is_empty());
        assert!(
            name.ends_with(".csv") || name.ends_with(".isf") || name.ends_with(".wfm"),
            "name={name}"
        );
        // Captured source must be parseable by the same path the GUI uses.
        let ext = name.rsplit('.').next().unwrap_or("csv");
        assert!(
            crate::waveform_file::load_waveform_bytes(&src, ext, "CH1").is_ok(),
            "demo waveform source should parse ({name})"
        );

        let source = InstrumentDevice::connect_demo(InstrumentKind::DcSource).unwrap();
        assert_eq!(source.profile.capabilities.channels, 3);
        let readings = InstrumentDevice::connect_demo(InstrumentKind::DcSource)
            .unwrap()
            .read_measurements()
            .unwrap();
        assert!(readings.len() >= 3);
    }

    #[test]
    fn normalize_wave_unit_maps_current_probe_tokens() {
        assert_eq!(normalize_wave_unit("A", "V"), "A");
        assert_eq!(normalize_wave_unit("\"Amps\"", "V"), "A");
        assert_eq!(normalize_wave_unit("V", "A"), "V");
        assert_eq!(normalize_wave_unit("", "V"), "V");
    }

    #[test]
    fn keysight_preamble_parses_points_and_scale() {
        let pre = "0,0,1000,1,1.000000E-08,-5.000000E-06,0,1.562500E-03,-4.000000E+00,128";
        let (n, xinc, xorig, xref, yinc, yorig, yref) = parse_keysight_preamble(pre).unwrap();
        assert_eq!(n, 1000);
        assert!((xinc - 1e-8).abs() < 1e-20);
        assert!((xorig + 5e-6).abs() < 1e-12);
        assert_eq!(xref, 0.0);
        assert!((yinc - 1.5625e-3).abs() < 1e-12);
        assert!((yorig + 4.0).abs() < 1e-9);
        assert_eq!(yref, 128.0);
        let v = scale_scope_y(YScaleKind::Keysight, 128.0, yinc, yorig, yref);
        assert!((v - yorig).abs() < 1e-12);
    }

    #[test]
    fn tek_wfm_source_validator_accepts_001_and_003() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sample_waveforms/Tektronix_WFM");
        let sample = root.join("analog_waveform.wfm");
        if sample.is_file() {
            let bytes = std::fs::read(&sample).unwrap();
            assert!(tek_waveform_source_bytes_ok(&bytes, "wfm"));
            assert!(!tek_waveform_source_bytes_ok(&bytes, "isf"));
        }
        // Legacy WiParse WFM#001 (no leading ':')
        let mut legacy = vec![0u8; 900];
        legacy[0] = 0x0f;
        legacy[1] = 0x0f;
        legacy[2..10].copy_from_slice(b"WFM#001\0");
        // Not a full valid file — validator must require a successful parse.
        assert!(!tek_waveform_source_bytes_ok(&legacy, "wfm"));
    }

    #[test]
    fn reads_source_measurements() {
        let mock = MockTransport::scripted([
            ("*IDN?", "RIGOL,DP832,SN,1"),
            ("MEAS:VOLT? CH1", "5"),
            ("MEAS:CURR? CH1", "2"),
            ("MEAS:VOLT? CH2", "0"),
            ("MEAS:CURR? CH2", "0"),
            ("MEAS:VOLT? CH3", "0"),
            ("MEAS:CURR? CH3", "0"),
        ]);
        let mut device = InstrumentDevice::from_transport(
            "MOCK",
            Box::new(mock),
            Some(InstrumentKind::DcSource),
        )
        .unwrap();
        assert_eq!(device.profile.capabilities.channels, 3);
        let values = device.read_measurements().unwrap();
        assert_eq!(values[2].value, 10.0); // CH1 Power
        assert_eq!(values[2].channel, "CH1 Power");
    }

    #[test]
    fn humanizes_scope_readings_without_scientific_notation() {
        assert_eq!(
            format_human_scope_reading(9.99e7, ScopeMeasType::Frequency, "Hz"),
            "99.9 MHz"
        );
        assert_eq!(
            format_human_scope_reading(1.25e-6, ScopeMeasType::Period, "s"),
            "1.25 µs"
        );
        assert_eq!(
            format_human_scope_reading(0.0033, ScopeMeasType::Pk2Pk, "V"),
            "3.3 mV"
        );
        assert_eq!(
            humanize_scope_reading_text("9.9900000E+7 \"Hz\"", ScopeMeasType::Frequency),
            "99.9 MHz"
        );
        assert_eq!(
            humanize_scope_reading_text("48.2", ScopeMeasType::DutyCycle),
            "48.2 %"
        );
    }
}
