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

    pub fn read_scope_waveform(
        &mut self,
        channel: u8,
        max_points: usize,
    ) -> Result<WaveformTrace, InstrumentError> {
        self.require(InstrumentKind::Oscilloscope, "waveform")?;
        let channel = channel.clamp(1, 8);
        let points = max_points.clamp(100, 1_000_000);
        let (xincr, xzero, ymult, yoff, yzero, signed) = if self.vendor_is("RIGOL")
            || self.vendor_is("SIGLENT")
        {
            self.session
                .write(&format!(":WAV:SOUR CHAN{channel}"))?;
            self.session.write(":WAV:MODE NORM")?;
            self.session.write(":WAV:FORM BYTE")?;
            // Cap transfer size before :WAV:DATA? (full memory depth can be millions of points).
            let _ = self.session.write(":WAV:STAR 1");
            let _ = self.session.write(&format!(":WAV:STOP {points}"));
            let _ = self.session.write(&format!(":WAV:POIN {points}"));
            (
                self.session.query_f64(":WAV:XINC?")?,
                self.session.query_f64(":WAV:XOR?").unwrap_or(0.0),
                self.session.query_f64(":WAV:YINC?")?,
                self.session.query_f64(":WAV:YOR?")?,
                self.session.query_f64(":WAV:YREF?").unwrap_or(0.0),
                false,
            )
        } else {
            // Tektronix / Keysight-style binary curve transfer.
            self.session
                .write(&format!("DATa:SOUrce CH{channel}"))?;
            self.session.write("DATa:ENCdg RIBINARY")?;
            self.session.write("DATa:WIDth 1")?;
            self.session.write("DATa:STARt 1")?;
            self.session.write(&format!("DATa:STOP {points}"))?;
            (
                self.session.query_f64("WFMOutpre:XINcr?")?,
                self.session.query_f64("WFMOutpre:XZEro?")?,
                self.session.query_f64("WFMOutpre:YMUlt?")?,
                self.session.query_f64("WFMOutpre:YOFf?")?,
                self.session.query_f64("WFMOutpre:YZEro?")?,
                true,
            )
        };
        if self.vendor_is("RIGOL") || self.vendor_is("SIGLENT") {
            self.session.write(":WAV:DATA?")?;
        } else {
            self.session.write("CURVe?")?;
        }
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
        let (x, y) = downsample_minmax(&x, &y, points.max(2));
        Ok(WaveformTrace {
            channel: format!("CH{channel}"),
            x,
            y,
            x_unit: "s".into(),
            y_unit: "V".into(),
        })
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
        let trace = scope.read_scope_waveform(1, 256).unwrap();
        assert!(trace.y.len() >= 64);

        let source = InstrumentDevice::connect_demo(InstrumentKind::DcSource).unwrap();
        assert_eq!(source.profile.capabilities.channels, 3);
        let readings = InstrumentDevice::connect_demo(InstrumentKind::DcSource)
            .unwrap()
            .read_measurements()
            .unwrap();
        assert!(readings.len() >= 3);
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
