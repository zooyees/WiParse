use crate::converter::ConverterPanel;
use crate::theme::{self as ui_theme, Tokens};
use egui::{Color32, CornerRadius, Frame, Margin, Pos2, Rect, Sense, Stroke, Vec2};
use std::f64::consts::PI;
use wiparse_core::i18n::Lang;

const CURVE_POINTS: usize = 801;
const RESONANCE_SPAN_HZ: f64 = 200_000.0;
const MIN_PLOT_FREQUENCY_HZ: f64 = 1.0;
const MIN_IMPEDANCE_OHM: f64 = 1.0e-15;
const Q_DECREMENT_FORMULA: &str = "δ = (1/N) · ln((V1−Bias)/(V2−Bias))";
const Q_DAMPING_FORMULA: &str = "ζ = δ / sqrt(4π² + δ²)";
const Q_EXACT_FORMULA: &str = "Q_exact = 1/(2ζ) = sqrt(4π² + δ²) / (2δ)";
const Q_APPROX_FORMULA: &str = "Q_approx ≈ π/δ = πN / ln((V1−Bias)/(V2−Bias))  (pi)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InductanceUnit {
    Microhenry,
    Millihenry,
    Henry,
}

impl InductanceUnit {
    const ALL: [Self; 3] = [Self::Microhenry, Self::Millihenry, Self::Henry];

    fn label(self) -> &'static str {
        match self {
            Self::Microhenry => "uH",
            Self::Millihenry => "mH",
            Self::Henry => "H",
        }
    }

    fn scale(self) -> f64 {
        match self {
            Self::Microhenry => 1.0e-6,
            Self::Millihenry => 1.0e-3,
            Self::Henry => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResistanceUnit {
    Milliohm,
    Ohm,
    Kiloohm,
    Megaohm,
}

impl ResistanceUnit {
    const ESR_UNITS: [Self; 3] = [Self::Milliohm, Self::Ohm, Self::Kiloohm];
    const FILTER_UNITS: [Self; 3] = [Self::Ohm, Self::Kiloohm, Self::Megaohm];
    const ALL: [Self; 4] = [Self::Milliohm, Self::Ohm, Self::Kiloohm, Self::Megaohm];

    fn label(self) -> &'static str {
        match self {
            Self::Milliohm => "mΩ",
            Self::Ohm => "Ω",
            Self::Kiloohm => "kΩ",
            Self::Megaohm => "MΩ",
        }
    }

    fn scale(self) -> f64 {
        match self {
            Self::Milliohm => 1.0e-3,
            Self::Ohm => 1.0,
            Self::Kiloohm => 1.0e3,
            Self::Megaohm => 1.0e6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimeUnit {
    Microsecond,
    Millisecond,
    Second,
}

impl TimeUnit {
    const ALL: [Self; 3] = [Self::Microsecond, Self::Millisecond, Self::Second];

    fn label(self) -> &'static str {
        match self {
            Self::Microsecond => "us",
            Self::Millisecond => "ms",
            Self::Second => "s",
        }
    }

    fn scale(self) -> f64 {
        match self {
            Self::Microsecond => 1.0e-6,
            Self::Millisecond => 1.0e-3,
            Self::Second => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapacitanceUnit {
    Picofarad,
    Nanofarad,
    Microfarad,
    Millifarad,
    Farad,
}

impl CapacitanceUnit {
    const LC_UNITS: [Self; 4] = [
        Self::Nanofarad,
        Self::Microfarad,
        Self::Millifarad,
        Self::Farad,
    ];
    const FILTER_UNITS: [Self; 5] = [
        Self::Picofarad,
        Self::Nanofarad,
        Self::Microfarad,
        Self::Millifarad,
        Self::Farad,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Picofarad => "pF",
            Self::Nanofarad => "nF",
            Self::Microfarad => "uF",
            Self::Millifarad => "mF",
            Self::Farad => "F",
        }
    }

    fn scale(self) -> f64 {
        match self {
            Self::Picofarad => 1.0e-12,
            Self::Nanofarad => 1.0e-9,
            Self::Microfarad => 1.0e-6,
            Self::Millifarad => 1.0e-3,
            Self::Farad => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CalcError {
    InvalidInductance,
    InvalidCapacitance,
    InvalidResistance,
    InvalidHighPass,
    InvalidLowPass,
    NoPassband,
    InvalidPeaks,
    InvalidInterval,
}

fn error_text(lang: Lang, error: CalcError) -> &'static str {
    match (lang, error) {
        (Lang::Zh, CalcError::InvalidInductance) => "电感必须是有限正值。",
        (Lang::En, CalcError::InvalidInductance) => "Inductance must be a finite positive value.",
        (Lang::Zh, CalcError::InvalidCapacitance) => "电容必须是有限正值。",
        (Lang::En, CalcError::InvalidCapacitance) => "Capacitance must be a finite positive value.",
        (Lang::Zh, CalcError::InvalidResistance) => "ESR 必须是有限非负值。",
        (Lang::En, CalcError::InvalidResistance) => "ESR must be finite and non-negative.",
        (Lang::Zh, CalcError::InvalidHighPass) => "高通 R_HP、C_HP 必须是有限正值。",
        (Lang::En, CalcError::InvalidHighPass) => {
            "High-pass R_HP and C_HP must be finite positive values."
        }
        (Lang::Zh, CalcError::InvalidLowPass) => "低通 R_LP、C_LP 必须是有限正值。",
        (Lang::En, CalcError::InvalidLowPass) => {
            "Low-pass R_LP and C_LP must be finite positive values."
        }
        (Lang::Zh, CalcError::NoPassband) => "f_L ≥ f_H，当前参数没有有效通带。",
        (Lang::En, CalcError::NoPassband) => {
            "f_L ≥ f_H; these values do not form a valid passband."
        }
        (Lang::Zh, CalcError::InvalidPeaks) => "峰值必须满足有限且 (V1−Bias) > (V2−Bias) > 0。",
        (Lang::En, CalcError::InvalidPeaks) => {
            "Peaks must be finite and satisfy (V1−Bias) > (V2−Bias) > 0."
        },
        (Lang::Zh, CalcError::InvalidInterval) => "N 必须是正整数。",
        (Lang::En, CalcError::InvalidInterval) => "N must be a positive integer.",
    }
}

fn positive_scaled(value: f64, scale: f64, error: CalcError) -> Result<f64, CalcError> {
    let scaled = value * scale;
    if value.is_finite() && value > 0.0 && scaled.is_finite() && scaled > 0.0 {
        Ok(scaled)
    } else {
        Err(error)
    }
}

fn nonnegative_scaled(value: f64, scale: f64, error: CalcError) -> Result<f64, CalcError> {
    let scaled = value * scale;
    if value.is_finite() && value >= 0.0 && scaled.is_finite() && scaled >= 0.0 {
        Ok(scaled)
    } else {
        Err(error)
    }
}

fn inductance_h(value: f64, unit: InductanceUnit) -> Result<f64, CalcError> {
    positive_scaled(value, unit.scale(), CalcError::InvalidInductance)
}

fn capacitance_f(value: f64, unit: CapacitanceUnit) -> Result<f64, CalcError> {
    positive_scaled(value, unit.scale(), CalcError::InvalidCapacitance)
}

fn resistance_ohm(value: f64, unit: ResistanceUnit, allow_zero: bool) -> Result<f64, CalcError> {
    if allow_zero {
        nonnegative_scaled(value, unit.scale(), CalcError::InvalidResistance)
    } else {
        positive_scaled(value, unit.scale(), CalcError::InvalidResistance)
    }
}

fn resonance_frequency(l_h: f64, c_f: f64) -> Result<f64, CalcError> {
    if !l_h.is_finite() || l_h <= 0.0 {
        return Err(CalcError::InvalidInductance);
    }
    if !c_f.is_finite() || c_f <= 0.0 {
        return Err(CalcError::InvalidCapacitance);
    }
    let frequency = 1.0 / (2.0 * PI * (l_h * c_f).sqrt());
    if frequency.is_finite() && frequency > 0.0 {
        Ok(frequency)
    } else {
        Err(CalcError::InvalidCapacitance)
    }
}

fn series_rlc_current(frequency_hz: f64, l_h: f64, c_f: f64, r_ohm: f64) -> f64 {
    if !frequency_hz.is_finite()
        || frequency_hz <= 0.0
        || !l_h.is_finite()
        || l_h <= 0.0
        || !c_f.is_finite()
        || c_f <= 0.0
        || !r_ohm.is_finite()
        || r_ohm < 0.0
    {
        return 0.0;
    }
    let omega = 2.0 * PI * frequency_hz;
    let reactance = omega * l_h - 1.0 / (omega * c_f);
    let impedance = r_ohm.hypot(reactance).max(MIN_IMPEDANCE_OHM);
    1.0 / impedance
}

fn resonance_frequency_bounds(f0_hz: f64) -> (f64, f64) {
    (
        (f0_hz - RESONANCE_SPAN_HZ).max(MIN_PLOT_FREQUENCY_HZ),
        f0_hz + RESONANCE_SPAN_HZ,
    )
}

fn resonance_curve(f0_hz: f64, l_h: f64, c_f: f64, r_ohm: f64) -> Vec<[f64; 2]> {
    let (low, high) = resonance_frequency_bounds(f0_hz);
    let mut frequencies: Vec<f64> = (0..CURVE_POINTS)
        .map(|index| low + (high - low) * index as f64 / (CURVE_POINTS - 1) as f64)
        .collect();
    frequencies.push(f0_hz);
    frequencies.sort_by(f64::total_cmp);
    frequencies.dedup_by(|left, right| (*left - *right).abs() <= f64::EPSILON * f0_hz);
    frequencies
        .into_iter()
        .map(|frequency| [frequency, series_rlc_current(frequency, l_h, c_f, r_ohm)])
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct BandpassResult {
    f_low_hz: f64,
    f_high_hz: f64,
    bandwidth_hz: f64,
    center_hz: f64,
    q: f64,
}

fn rc_cutoff(r_ohm: f64, c_f: f64, error: CalcError) -> Result<f64, CalcError> {
    if !r_ohm.is_finite() || r_ohm <= 0.0 || !c_f.is_finite() || c_f <= 0.0 {
        return Err(error);
    }
    let frequency = 1.0 / (2.0 * PI * r_ohm * c_f);
    if frequency.is_finite() && frequency > 0.0 {
        Ok(frequency)
    } else {
        Err(error)
    }
}

fn bandpass_result(
    r_hp: f64,
    c_hp: f64,
    r_lp: f64,
    c_lp: f64,
) -> Result<BandpassResult, CalcError> {
    let f_low_hz = rc_cutoff(r_hp, c_hp, CalcError::InvalidHighPass)?;
    let f_high_hz = rc_cutoff(r_lp, c_lp, CalcError::InvalidLowPass)?;
    if f_low_hz >= f_high_hz {
        return Err(CalcError::NoPassband);
    }
    let bandwidth_hz = f_high_hz - f_low_hz;
    let center_hz = (f_low_hz * f_high_hz).sqrt();
    let q = center_hz / bandwidth_hz;
    if [bandwidth_hz, center_hz, q]
        .iter()
        .all(|value| value.is_finite())
    {
        Ok(BandpassResult {
            f_low_hz,
            f_high_hz,
            bandwidth_hz,
            center_hz,
            q,
        })
    } else {
        Err(CalcError::NoPassband)
    }
}

#[derive(Debug, Clone, Copy)]
struct QResult {
    decrement: f64,
    damping_ratio: f64,
    exact_q: f64,
    approx_q: f64,
}

fn logarithmic_decrement_q(v1: f64, v2: f64, bias: f64, intervals: f64) -> Result<QResult, CalcError> {
    if !v1.is_finite() || !v2.is_finite() || !bias.is_finite() {
        return Err(CalcError::InvalidPeaks);
    }
    let a = v1 - bias;
    let b = v2 - bias;
    if a <= b || b <= 0.0 {
        return Err(CalcError::InvalidPeaks);
    }
    if !intervals.is_finite() || intervals <= 0.0 || intervals.fract().abs() > f64::EPSILON {
        return Err(CalcError::InvalidInterval);
    }
    let log_ratio = (a / b).ln();
    let decrement = log_ratio / intervals;
    let root = (4.0 * PI * PI + decrement * decrement).sqrt();
    let damping_ratio = decrement / root;
    let exact_q = root / (2.0 * decrement);
    let approx_q = PI * intervals / log_ratio;
    if [decrement, damping_ratio, exact_q, approx_q]
        .iter()
        .all(|value| value.is_finite() && *value > 0.0)
    {
        Ok(QResult {
            decrement,
            damping_ratio,
            exact_q,
            approx_q,
        })
    } else {
        Err(CalcError::InvalidPeaks)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RcResult {
    tau_s: f64,
    capacitor_voltage_v: f64,
}

fn rc_time_response(
    resistance_ohm: f64,
    capacitance_f: f64,
    initial_voltage_v: f64,
    final_voltage_v: f64,
    time_s: f64,
) -> Result<RcResult, &'static str> {
    if !resistance_ohm.is_finite() || resistance_ohm <= 0.0 {
        return Err("resistance");
    }
    if !capacitance_f.is_finite() || capacitance_f <= 0.0 {
        return Err("capacitance");
    }
    if !initial_voltage_v.is_finite() || !final_voltage_v.is_finite() {
        return Err("voltage");
    }
    if !time_s.is_finite() || time_s < 0.0 {
        return Err("time");
    }
    let tau_s = resistance_ohm * capacitance_f;
    if !tau_s.is_finite() || tau_s <= 0.0 {
        return Err("time_constant");
    }
    let decay = (-time_s / tau_s).exp();
    let capacitor_voltage_v = final_voltage_v + (initial_voltage_v - final_voltage_v) * decay;
    if capacitor_voltage_v.is_finite() {
        Ok(RcResult {
            tau_s,
            capacitor_voltage_v,
        })
    } else {
        Err("result")
    }
}

fn rc_settling_percentages(tau_multiple: f64) -> (f64, f64) {
    let remaining = (-tau_multiple).exp() * 100.0;
    (100.0 - remaining, remaining)
}

fn time_seconds(value: f64, unit: TimeUnit) -> Result<f64, &'static str> {
    let seconds = value * unit.scale();
    if value.is_finite() && value >= 0.0 && seconds.is_finite() && seconds >= 0.0 {
        Ok(seconds)
    } else {
        Err("time")
    }
}

fn format_time(seconds: f64) -> String {
    if seconds >= 1.0 {
        format!("{seconds:.5} s")
    } else if seconds >= 1.0e-3 {
        format!("{:.5} ms", seconds * 1.0e3)
    } else if seconds >= 1.0e-6 {
        format!("{:.5} us", seconds * 1.0e6)
    } else {
        format!("{:.5} ns", seconds * 1.0e9)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CrcParams {
    width: u8,
    poly: u32,
    init: u32,
    refin: bool,
    refout: bool,
    xorout: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrcPreset {
    Crc8,
    Crc8Maxim,
    Crc16Arc,
    Crc16Modbus,
    Crc16CcittFalse,
    Crc16Xmodem,
    Crc32IsoHdlc,
    Crc32C,
    Custom,
}

impl CrcPreset {
    const ALL: [Self; 9] = [
        Self::Crc8,
        Self::Crc8Maxim,
        Self::Crc16Arc,
        Self::Crc16Modbus,
        Self::Crc16CcittFalse,
        Self::Crc16Xmodem,
        Self::Crc32IsoHdlc,
        Self::Crc32C,
        Self::Custom,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Crc8 => "CRC-8",
            Self::Crc8Maxim => "CRC-8/MAXIM-DOW",
            Self::Crc16Arc => "CRC-16/IBM (ARC)",
            Self::Crc16Modbus => "CRC-16/MODBUS",
            Self::Crc16CcittFalse => "CRC-16/CCITT-FALSE",
            Self::Crc16Xmodem => "CRC-16/XMODEM",
            Self::Crc32IsoHdlc => "CRC-32/ISO-HDLC",
            Self::Crc32C => "CRC-32C",
            Self::Custom => "Custom",
        }
    }

    fn params(self) -> Option<CrcParams> {
        Some(match self {
            Self::Crc8 => CrcParams {
                width: 8,
                poly: 0x07,
                init: 0,
                refin: false,
                refout: false,
                xorout: 0,
            },
            Self::Crc8Maxim => CrcParams {
                width: 8,
                poly: 0x31,
                init: 0,
                refin: true,
                refout: true,
                xorout: 0,
            },
            Self::Crc16Arc => CrcParams {
                width: 16,
                poly: 0x8005,
                init: 0,
                refin: true,
                refout: true,
                xorout: 0,
            },
            Self::Crc16Modbus => CrcParams {
                width: 16,
                poly: 0x8005,
                init: 0xFFFF,
                refin: true,
                refout: true,
                xorout: 0,
            },
            Self::Crc16CcittFalse => CrcParams {
                width: 16,
                poly: 0x1021,
                init: 0xFFFF,
                refin: false,
                refout: false,
                xorout: 0,
            },
            Self::Crc16Xmodem => CrcParams {
                width: 16,
                poly: 0x1021,
                init: 0,
                refin: false,
                refout: false,
                xorout: 0,
            },
            Self::Crc32IsoHdlc => CrcParams {
                width: 32,
                poly: 0x04C11DB7,
                init: 0xFFFF_FFFF,
                refin: true,
                refout: true,
                xorout: 0xFFFF_FFFF,
            },
            Self::Crc32C => CrcParams {
                width: 32,
                poly: 0x1EDC6F41,
                init: 0xFFFF_FFFF,
                refin: true,
                refout: true,
                xorout: 0xFFFF_FFFF,
            },
            Self::Custom => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrcInputMode {
    Hex,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrcError {
    InvalidHexData,
    InvalidWidth,
    InvalidPoly,
    InvalidInit,
    InvalidXorout,
}

fn crc_mask(width: u8) -> Option<u32> {
    match width {
        8 => Some(0xFF),
        16 => Some(0xFFFF),
        32 => Some(u32::MAX),
        _ => None,
    }
}

fn validate_crc_params(params: CrcParams) -> Result<CrcParams, CrcError> {
    let mask = crc_mask(params.width).ok_or(CrcError::InvalidWidth)?;
    if params.poly == 0 || params.poly > mask {
        return Err(CrcError::InvalidPoly);
    }
    if params.init > mask {
        return Err(CrcError::InvalidInit);
    }
    if params.xorout > mask {
        return Err(CrcError::InvalidXorout);
    }
    Ok(params)
}

fn reflect_bits(mut value: u32, width: u8) -> u32 {
    let mut reflected = 0;
    for _ in 0..width {
        reflected = (reflected << 1) | (value & 1);
        value >>= 1;
    }
    reflected
}

fn calculate_crc(data: &[u8], params: CrcParams) -> Result<u32, CrcError> {
    let params = validate_crc_params(params)?;
    let mask = crc_mask(params.width).ok_or(CrcError::InvalidWidth)?;
    let mut crc = params.init;
    if params.refin {
        let reflected_poly = reflect_bits(params.poly, params.width);
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ reflected_poly
                } else {
                    crc >> 1
                };
            }
        }
    } else {
        let top_bit = 1_u32 << (params.width - 1);
        for byte in data {
            crc ^= u32::from(*byte) << (params.width - 8);
            for _ in 0..8 {
                crc = if crc & top_bit != 0 {
                    (crc << 1) ^ params.poly
                } else {
                    crc << 1
                } & mask;
            }
        }
    }
    if params.refout != params.refin {
        crc = reflect_bits(crc, params.width);
    }
    Ok((crc ^ params.xorout) & mask)
}

fn parse_hex_data(input: &str) -> Result<Vec<u8>, CrcError> {
    let normalized = input.replace([',', ';'], " ");
    let mut bytes = Vec::new();
    for token in normalized.split_whitespace() {
        let digits = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
            .unwrap_or(token);
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CrcError::InvalidHexData);
        }
        if digits.len() <= 2 {
            bytes.push(u8::from_str_radix(digits, 16).map_err(|_| CrcError::InvalidHexData)?);
            continue;
        }
        if digits.len() % 2 != 0 {
            return Err(CrcError::InvalidHexData);
        }
        for offset in (0..digits.len()).step_by(2) {
            bytes.push(
                u8::from_str_radix(&digits[offset..offset + 2], 16)
                    .map_err(|_| CrcError::InvalidHexData)?,
            );
        }
    }
    Ok(bytes)
}

fn parse_crc_hex(input: &str, width: u8, error: CrcError) -> Result<u32, CrcError> {
    let digits = input
        .trim()
        .strip_prefix("0x")
        .or_else(|| input.trim().strip_prefix("0X"))
        .unwrap_or(input.trim())
        .replace('_', "");
    let value = u32::from_str_radix(&digits, 16).map_err(|_| error)?;
    if value <= crc_mask(width).ok_or(CrcError::InvalidWidth)? {
        Ok(value)
    } else {
        Err(error)
    }
}

fn parse_number(text: &str, error: CalcError) -> Result<f64, CalcError> {
    crate::converter::parse_numeric_literal(text).map_err(|_| error)
}

fn format_frequency(hz: f64) -> String {
    if hz >= 1.0e6 {
        format!("{:.4} MHz", hz / 1.0e6)
    } else if hz >= 1.0e3 {
        format!("{:.4} kHz", hz / 1.0e3)
    } else {
        format!("{hz:.4} Hz")
    }
}

struct LcState {
    inductance: String,
    inductance_unit: InductanceUnit,
    inductor_esr: String,
    inductor_esr_unit: ResistanceUnit,
    capacitance: String,
    capacitance_unit: CapacitanceUnit,
    capacitor_esr: String,
    capacitor_esr_unit: ResistanceUnit,
    result: Option<LcResult>,
    error: Option<CalcError>,
}

struct LcResult {
    frequency_hz: f64,
    curve: Vec<[f64; 2]>,
}

impl Default for LcState {
    fn default() -> Self {
        Self {
            inductance: "10".into(),
            inductance_unit: InductanceUnit::Microhenry,
            inductor_esr: "50".into(),
            inductor_esr_unit: ResistanceUnit::Milliohm,
            capacitance: "100".into(),
            capacitance_unit: CapacitanceUnit::Nanofarad,
            capacitor_esr: "50".into(),
            capacitor_esr_unit: ResistanceUnit::Milliohm,
            result: None,
            error: None,
        }
    }
}

impl LcState {
    fn calculate(&mut self) {
        let result = (|| {
            let l = inductance_h(
                parse_number(&self.inductance, CalcError::InvalidInductance)?,
                self.inductance_unit,
            )?;
            let c = capacitance_f(
                parse_number(&self.capacitance, CalcError::InvalidCapacitance)?,
                self.capacitance_unit,
            )?;
            let r_l = resistance_ohm(
                parse_number(&self.inductor_esr, CalcError::InvalidResistance)?,
                self.inductor_esr_unit,
                true,
            )?;
            let r_c = resistance_ohm(
                parse_number(&self.capacitor_esr, CalcError::InvalidResistance)?,
                self.capacitor_esr_unit,
                true,
            )?;
            let frequency_hz = resonance_frequency(l, c)?;
            Ok(LcResult {
                frequency_hz,
                curve: resonance_curve(frequency_hz, l, c, r_l + r_c),
            })
        })();
        match result {
            Ok(value) => {
                self.result = Some(value);
                self.error = None;
            }
            Err(error) => {
                self.result = None;
                self.error = Some(error);
            }
        }
    }
}

struct BandpassState {
    r_hp: String,
    r_hp_unit: ResistanceUnit,
    c_hp: String,
    c_hp_unit: CapacitanceUnit,
    r_lp: String,
    r_lp_unit: ResistanceUnit,
    c_lp: String,
    c_lp_unit: CapacitanceUnit,
    result: Option<BandpassResult>,
    error: Option<CalcError>,
}

impl Default for BandpassState {
    fn default() -> Self {
        Self {
            r_hp: "10".into(),
            r_hp_unit: ResistanceUnit::Kiloohm,
            c_hp: "100".into(),
            c_hp_unit: CapacitanceUnit::Nanofarad,
            r_lp: "10".into(),
            r_lp_unit: ResistanceUnit::Kiloohm,
            c_lp: "1".into(),
            c_lp_unit: CapacitanceUnit::Nanofarad,
            result: None,
            error: None,
        }
    }
}

impl BandpassState {
    fn calculate(&mut self) {
        let result = (|| {
            let r_hp = positive_scaled(
                parse_number(&self.r_hp, CalcError::InvalidHighPass)?,
                self.r_hp_unit.scale(),
                CalcError::InvalidHighPass,
            )?;
            let c_hp = positive_scaled(
                parse_number(&self.c_hp, CalcError::InvalidHighPass)?,
                self.c_hp_unit.scale(),
                CalcError::InvalidHighPass,
            )?;
            let r_lp = positive_scaled(
                parse_number(&self.r_lp, CalcError::InvalidLowPass)?,
                self.r_lp_unit.scale(),
                CalcError::InvalidLowPass,
            )?;
            let c_lp = positive_scaled(
                parse_number(&self.c_lp, CalcError::InvalidLowPass)?,
                self.c_lp_unit.scale(),
                CalcError::InvalidLowPass,
            )?;
            bandpass_result(r_hp, c_hp, r_lp, c_lp)
        })();
        match result {
            Ok(value) => {
                self.result = Some(value);
                self.error = None;
            }
            Err(error) => {
                self.result = None;
                self.error = Some(error);
            }
        }
    }
}

struct QState {
    v1: String,
    v2: String,
    bias: String,
    intervals: String,
    result: Option<QResult>,
    error: Option<CalcError>,
}

impl Default for QState {
    fn default() -> Self {
        Self {
            v1: "10".into(),
            v2: "8".into(),
            bias: "0".into(),
            intervals: "1".into(),
            result: None,
            error: None,
        }
    }
}

impl QState {
    fn calculate(&mut self) {
        let result = (|| {
            let v1 = parse_number(&self.v1, CalcError::InvalidPeaks)?;
            let v2 = parse_number(&self.v2, CalcError::InvalidPeaks)?;
            let bias = parse_number(&self.bias, CalcError::InvalidPeaks)?;
            let intervals = parse_number(&self.intervals, CalcError::InvalidInterval)?;
            logarithmic_decrement_q(v1, v2, bias, intervals)
        })();
        match result {
            Ok(value) => {
                self.result = Some(value);
                self.error = None;
            }
            Err(error) => {
                self.result = None;
                self.error = Some(error);
            }
        }
    }
}

struct RcState {
    resistance: String,
    resistance_unit: ResistanceUnit,
    capacitance: String,
    capacitance_unit: CapacitanceUnit,
    initial_voltage: String,
    final_voltage: String,
    time: String,
    time_unit: TimeUnit,
    result: Option<RcResult>,
    error: Option<&'static str>,
}

impl Default for RcState {
    fn default() -> Self {
        Self {
            resistance: "10".into(),
            resistance_unit: ResistanceUnit::Kiloohm,
            capacitance: "100".into(),
            capacitance_unit: CapacitanceUnit::Microfarad,
            initial_voltage: "0".into(),
            final_voltage: "5".into(),
            time: "1".into(),
            time_unit: TimeUnit::Second,
            result: None,
            error: None,
        }
    }
}

impl RcState {
    fn calculate(&mut self) {
        let result = (|| {
            let resistance = positive_scaled(
                parse_number(&self.resistance, CalcError::InvalidResistance)
                    .map_err(|_| "resistance")?,
                self.resistance_unit.scale(),
                CalcError::InvalidResistance,
            )
            .map_err(|_| "resistance")?;
            let capacitance = positive_scaled(
                parse_number(&self.capacitance, CalcError::InvalidCapacitance)
                    .map_err(|_| "capacitance")?,
                self.capacitance_unit.scale(),
                CalcError::InvalidCapacitance,
            )
            .map_err(|_| "capacitance")?;
            let initial_voltage = crate::converter::parse_numeric_literal(&self.initial_voltage)
                .map_err(|_| "voltage")?;
            let final_voltage = crate::converter::parse_numeric_literal(&self.final_voltage)
                .map_err(|_| "voltage")?;
            let time = time_seconds(
                crate::converter::parse_numeric_literal(&self.time).map_err(|_| "time")?,
                self.time_unit,
            )?;
            rc_time_response(
                resistance,
                capacitance,
                initial_voltage,
                final_voltage,
                time,
            )
        })();
        match result {
            Ok(value) => {
                self.result = Some(value);
                self.error = None;
            }
            Err(error) => {
                self.result = None;
                self.error = Some(error);
            }
        }
    }
}

struct CrcState {
    data: String,
    input_mode: CrcInputMode,
    preset: CrcPreset,
    custom_width: u8,
    custom_poly: String,
    custom_init: String,
    custom_xorout: String,
    custom_refin: bool,
    custom_refout: bool,
    result: Option<u32>,
    error: Option<CrcError>,
}

impl Default for CrcState {
    fn default() -> Self {
        Self {
            data: "123456789".into(),
            input_mode: CrcInputMode::Text,
            preset: CrcPreset::Crc32IsoHdlc,
            custom_width: 16,
            custom_poly: "0x1021".into(),
            custom_init: "0xFFFF".into(),
            custom_xorout: "0x0000".into(),
            custom_refin: false,
            custom_refout: false,
            result: None,
            error: None,
        }
    }
}

impl CrcState {
    fn active_params(&self) -> Result<CrcParams, CrcError> {
        if let Some(params) = self.preset.params() {
            return Ok(params);
        }
        validate_crc_params(CrcParams {
            width: self.custom_width,
            poly: parse_crc_hex(&self.custom_poly, self.custom_width, CrcError::InvalidPoly)?,
            init: parse_crc_hex(&self.custom_init, self.custom_width, CrcError::InvalidInit)?,
            refin: self.custom_refin,
            refout: self.custom_refout,
            xorout: parse_crc_hex(
                &self.custom_xorout,
                self.custom_width,
                CrcError::InvalidXorout,
            )?,
        })
    }

    fn calculate(&mut self) {
        let result = (|| {
            let data = match self.input_mode {
                CrcInputMode::Hex => parse_hex_data(&self.data)?,
                CrcInputMode::Text => self.data.as_bytes().to_vec(),
            };
            let params = self.active_params()?;
            Ok((calculate_crc(&data, params)?, params.width))
        })();
        match result {
            Ok((value, _width)) => {
                self.result = Some(value);
                self.error = None;
            }
            Err(error) => {
                self.result = None;
                self.error = Some(error);
            }
        }
    }
}

pub struct CalculatorPanel {
    lc: LcState,
    bandpass: BandpassState,
    q: QState,
    rc: RcState,
    crc: CrcState,
    converter: ConverterPanel,
}

impl CalculatorPanel {
    pub fn new() -> Self {
        Self {
            lc: LcState::default(),
            bandpass: BandpassState::default(),
            q: QState::default(),
            rc: RcState::default(),
            crc: CrcState::default(),
            converter: ConverterPanel::new(),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        let available = ui.available_rect_before_wrap();
        let gap = 8.0;
        let cell_width = ((available.width() - gap * 2.0) / 3.0).max(1.0);
        let cell_height = ((available.height() - gap) / 2.0).max(1.0);

        for index in 0..6 {
            let row = index / 3;
            let column = index % 3;
            let min = Pos2::new(
                available.left() + column as f32 * (cell_width + gap),
                available.top() + row as f32 * (cell_height + gap),
            );
            let rect = Rect::from_min_size(min, Vec2::new(cell_width, cell_height));
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                Frame::NONE
                    .fill(t.panel_bg)
                    .stroke(Stroke::new(1.0_f32, t.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_min_size(Vec2::new(
                            (cell_width - 20.0).max(1.0),
                            (cell_height - 20.0).max(1.0),
                        ));
                        egui::ScrollArea::vertical()
                            .id_salt(("calculator_tool", index))
                            .auto_shrink([false, false])
                            .show(ui, |ui| match index {
                                0 => self.lc_ui(ui, lang, t),
                                1 => self.bandpass_ui(ui, lang, t),
                                2 => self.q_ui(ui, lang, t),
                                3 => self.rc_ui(ui, lang, t),
                                4 => self.crc_ui(ui, lang, t),
                                5 => self.converter.ui(ui, lang, t),
                                _ => placeholder_ui(ui, lang, t, index + 1),
                            });
                    });
            });
        }
        ui.allocate_rect(available, Sense::hover());
    }

    fn lc_ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        tool_header(
            ui,
            t,
            1,
            text(lang, "LC 谐振频率", "LC Resonance"),
            text(
                lang,
                "串联 LC 谐振与谐振腔电流",
                "Series LC resonance and tank current",
            ),
        );
        let frequency = self
            .lc
            .result
            .as_ref()
            .map(|result| format_frequency(result.frequency_hz))
            .unwrap_or_else(|| "—".into());
        ui.columns(2, |columns| {
            section_heading(
                &mut columns[0],
                t,
                text(lang, "公式与说明", "Formula & notes"),
            );
            formula_box(
                &mut columns[0],
                t,
                &[
                    "f₀ = 1 / (2π√LC)",
                    "I(f) = 1 / √((R_L+R_C)² + (2πfL−1/(2πfC))²)",
                    text(
                        lang,
                        "曲线采用 1 Vrms 激励，并以 f₀±200 kHz 线性取样。",
                        "The 1 Vrms curve uses a linear f₀ ± 200 kHz span.",
                    ),
                ],
            );
            section_heading(&mut columns[0], t, text(lang, "结果", "Result"));
            result_table(&mut columns[0], t, &[("f₀", frequency)]);

            section_heading(&mut columns[1], t, text(lang, "参数", "Parameters"));
            let metrics = input_grid_metrics(columns[1].available_width());
            egui::Grid::new("lc_parameter_grid")
                .num_columns(3)
                .spacing([metrics.gap, 5.0])
                .min_row_height(26.0)
                .show(&mut columns[1], |ui| {
                    compact_grid_row(
                        ui,
                        metrics,
                        text(lang, "电感 L", "L value"),
                        &mut self.lc.inductance,
                        &mut self.lc.inductance_unit,
                        &InductanceUnit::ALL,
                        |unit| unit.label(),
                        "lc_l_unit",
                    );
                    compact_grid_row(
                        ui,
                        metrics,
                        text(lang, "电感 ESR", "L ESR"),
                        &mut self.lc.inductor_esr,
                        &mut self.lc.inductor_esr_unit,
                        &ResistanceUnit::ESR_UNITS,
                        |unit| unit.label(),
                        "lc_rl_unit",
                    );
                    compact_grid_row(
                        ui,
                        metrics,
                        text(lang, "电容 C", "C value"),
                        &mut self.lc.capacitance,
                        &mut self.lc.capacitance_unit,
                        &CapacitanceUnit::LC_UNITS,
                        |unit| unit.label(),
                        "lc_c_unit",
                    );
                    compact_grid_row(
                        ui,
                        metrics,
                        text(lang, "电容 ESR", "C ESR"),
                        &mut self.lc.capacitor_esr,
                        &mut self.lc.capacitor_esr_unit,
                        &ResistanceUnit::ESR_UNITS,
                        |unit| unit.label(),
                        "lc_rc_unit",
                    );
                });
            if compact_calculate_button(
                &mut columns[1],
                t,
                text(lang, "计算", "Calculate"),
                metrics,
            ) {
                self.lc.calculate();
            }
            error_slot(
                &mut columns[1],
                self.lc.error.map(|error| error_text(lang, error)),
            );
        });

        if let Some(result) = &self.lc.result {
            section_heading(
                ui,
                t,
                text(
                    lang,
                    "频率 - 谐振电流（1 Vrms）",
                    "Frequency - current (1 Vrms)",
                ),
            );
            current_plot(ui, t, &result.curve);
        }
    }

    fn bandpass_ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        tool_header(
            ui,
            t,
            2,
            text(lang, "带通滤波器", "Band-pass Filter"),
            text(
                lang,
                "独立一阶 RC 高通 + 低通级",
                "Independent 1st-order RC high/low-pass stages",
            ),
        );
        let rows = if let Some(result) = self.bandpass.result {
            vec![
                ("f_L", format_frequency(result.f_low_hz)),
                ("f_H", format_frequency(result.f_high_hz)),
                ("BW", format_frequency(result.bandwidth_hz)),
                ("f₀", format_frequency(result.center_hz)),
                ("Q", format!("{:.5}", result.q)),
            ]
        } else {
            vec![
                ("f_L", "—".into()),
                ("f_H", "—".into()),
                ("BW", "—".into()),
                ("f₀", "—".into()),
                ("Q", "—".into()),
            ]
        };
        ui.columns(2, |columns| {
            section_heading(&mut columns[0], t, text(lang, "参数", "Parameters"));
            let metrics = input_grid_metrics(columns[0].available_width());
            egui::Grid::new("bandpass_parameter_grid")
                .num_columns(3)
                .spacing([metrics.gap, 5.0])
                .min_row_height(26.0)
                .show(&mut columns[0], |ui| {
                    compact_grid_row(
                        ui,
                        metrics,
                        "C1 (HP)",
                        &mut self.bandpass.c_hp,
                        &mut self.bandpass.c_hp_unit,
                        &CapacitanceUnit::FILTER_UNITS,
                        |unit| unit.label(),
                        "bp_chp_unit",
                    );
                    compact_grid_row(
                        ui,
                        metrics,
                        "R1 (HP)",
                        &mut self.bandpass.r_hp,
                        &mut self.bandpass.r_hp_unit,
                        &ResistanceUnit::FILTER_UNITS,
                        |unit| unit.label(),
                        "bp_rhp_unit",
                    );
                    compact_grid_row(
                        ui,
                        metrics,
                        "R2 (LP)",
                        &mut self.bandpass.r_lp,
                        &mut self.bandpass.r_lp_unit,
                        &ResistanceUnit::FILTER_UNITS,
                        |unit| unit.label(),
                        "bp_rlp_unit",
                    );
                    compact_grid_row(
                        ui,
                        metrics,
                        "C2 (LP)",
                        &mut self.bandpass.c_lp,
                        &mut self.bandpass.c_lp_unit,
                        &CapacitanceUnit::FILTER_UNITS,
                        |unit| unit.label(),
                        "bp_clp_unit",
                    );
                });
            if compact_calculate_button(
                &mut columns[0],
                t,
                text(lang, "计算", "Calculate"),
                metrics,
            ) {
                self.bandpass.calculate();
            }
            error_slot(
                &mut columns[0],
                self.bandpass.error.map(|error| error_text(lang, error)),
            );

            section_heading(&mut columns[1], t, text(lang, "结果", "Results"));
            result_table(&mut columns[1], t, &rows);
        });

        section_heading(ui, t, text(lang, "公式与说明", "Formula & notes"));
        formula_box(
            ui,
            t,
            &[
                "f_L=1/(2πR1·C1)  ·  f_H=1/(2πR2·C2)",
                "BW=f_H−f_L  ·  f₀=√(f_L f_H)  ·  Q=f₀/BW",
                text(
                    lang,
                    "理想缓冲隔离两级，因此按独立 RC 级计算。",
                    "An ideal buffer isolates the independently calculated RC stages.",
                ),
            ],
        );
        section_heading(ui, t, text(lang, "电路结构", "Circuit"));
        bandpass_schematic(ui, t, lang);
    }

    fn q_ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        tool_header(
            ui,
            t,
            3,
            text(lang, "Q 值（峰值衰减）", "Q from Peak Decay"),
            text(
                lang,
                "相隔 N 个完整振荡周期的峰值对数减量法",
                "Log decrement across N complete oscillation cycles",
            ),
        );
        let rows = if let Some(result) = self.q.result {
            vec![
                ("δ", format!("{:.6}", result.decrement)),
                ("ζ", format!("{:.6}", result.damping_ratio)),
                (
                    text(lang, "精确 Q", "Exact Q"),
                    format!("{:.5}", result.exact_q),
                ),
                (
                    text(lang, "近似 Q", "Approx Q"),
                    format!("{:.5}", result.approx_q),
                ),
            ]
        } else {
            vec![
                ("δ", "—".into()),
                ("ζ", "—".into()),
                (text(lang, "精确 Q", "Exact Q"), "—".into()),
                (text(lang, "近似 Q", "Approx Q"), "—".into()),
            ]
        };
        ui.columns(2, |columns| {
            section_heading(&mut columns[0], t, text(lang, "参数", "Parameters"));
            let metrics = input_grid_metrics(columns[0].available_width());
            egui::Grid::new("q_parameter_grid")
                .num_columns(3)
                .spacing([metrics.gap, 5.0])
                .min_row_height(26.0)
                .show(&mut columns[0], |ui| {
                    compact_plain_grid_row(
                        ui,
                        metrics,
                        "V1",
                        &mut self.q.v1,
                        text(lang, "同单位", "same"),
                    );
                    compact_plain_grid_row(
                        ui,
                        metrics,
                        "V2",
                        &mut self.q.v2,
                        text(lang, "同单位", "same"),
                    );
                    compact_plain_grid_row(
                        ui,
                        metrics,
                        text(lang, "偏置", "Bias"),
                        &mut self.q.bias,
                        text(lang, "同单位", "same"),
                    );
                    compact_plain_grid_row(
                        ui,
                        metrics,
                        "N",
                        &mut self.q.intervals,
                        text(lang, "完整周期", "cycles"),
                    );
                });
            if compact_calculate_button(
                &mut columns[0],
                t,
                text(lang, "计算", "Calculate"),
                metrics,
            ) {
                self.q.calculate();
            }
            error_slot(
                &mut columns[0],
                self.q.error.map(|error| error_text(lang, error)),
            );

            section_heading(&mut columns[1], t, text(lang, "结果", "Results"));
            result_table(&mut columns[1], t, &rows);
        });

        section_heading(ui, t, text(lang, "公式与说明", "Formula & notes"));
        formula_box(
            ui,
            t,
            &[
                Q_DECREMENT_FORMULA,
                Q_DAMPING_FORMULA,
                Q_EXACT_FORMULA,
                Q_APPROX_FORMULA,
                text(
                    lang,
                    "主结果采用严格关系；Q_approx 仅适用于 δ 较小（高 Q）。N 是同极性峰值间完整阻尼振荡周期数。",
                    "Exact Q is primary; Q_approx is only for small δ (high Q). N counts complete damped cycles between same-polarity peaks.",
                ),
            ],
        );
    }

    fn rc_ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        tool_header(
            ui,
            t,
            4,
            text(lang, "RC 时间常数", "RC Time Constant"),
            text(
                lang,
                "R_th 与电容构成的单极点通用阶跃响应",
                "Single-pole step response using R_th and C",
            ),
        );
        let rows = if let Some(result) = self.rc.result {
            vec![
                ("τ", format_time(result.tau_s)),
                ("Vc(t)", format!("{:.6} V", result.capacitor_voltage_v)),
            ]
        } else {
            vec![("τ", "—".into()), ("Vc(t)", "—".into())]
        };
        ui.columns(2, |columns| {
            section_heading(&mut columns[0], t, text(lang, "参数", "Parameters"));
            let metrics = input_grid_metrics(columns[0].available_width());
            egui::Grid::new("rc_parameter_grid")
                .num_columns(3)
                .spacing([metrics.gap, 5.0])
                .min_row_height(26.0)
                .show(&mut columns[0], |ui| {
                    compact_grid_row(
                        ui,
                        metrics,
                        "R_th",
                        &mut self.rc.resistance,
                        &mut self.rc.resistance_unit,
                        &ResistanceUnit::ALL,
                        |unit| unit.label(),
                        "rc_r_unit",
                    );
                    compact_grid_row(
                        ui,
                        metrics,
                        "C",
                        &mut self.rc.capacitance,
                        &mut self.rc.capacitance_unit,
                        &CapacitanceUnit::FILTER_UNITS,
                        |unit| unit.label(),
                        "rc_c_unit",
                    );
                    compact_plain_grid_row(
                        ui,
                        metrics,
                        "V_initial",
                        &mut self.rc.initial_voltage,
                        "V",
                    );
                    compact_plain_grid_row(ui, metrics, "V_final", &mut self.rc.final_voltage, "V");
                    compact_grid_row(
                        ui,
                        metrics,
                        "t",
                        &mut self.rc.time,
                        &mut self.rc.time_unit,
                        &TimeUnit::ALL,
                        |unit| unit.label(),
                        "rc_t_unit",
                    );
                });
            if compact_calculate_button(
                &mut columns[0],
                t,
                text(lang, "计算", "Calculate"),
                metrics,
            ) {
                self.rc.calculate();
            }
            error_slot(
                &mut columns[0],
                self.rc.error.map(|error| rc_error_text(lang, error)),
            );

            section_heading(&mut columns[1], t, text(lang, "结果", "Results"));
            result_table(&mut columns[1], t, &rows);
        });

        section_heading(ui, t, text(lang, "公式与说明", "Formula & notes"));
        let (complete_1, remaining_1) = rc_settling_percentages(1.0);
        let (complete_3, remaining_3) = rc_settling_percentages(3.0);
        let (complete_5, remaining_5) = rc_settling_percentages(5.0);
        let settling_note = match lang {
            Lang::Zh => format!(
                "1τ: 完成{complete_1:.3}%/误差{remaining_1:.3}% · 3τ: {complete_3:.3}%/{remaining_3:.3}% · 5τ: {complete_5:.3}%/{remaining_5:.3}%"
            ),
            Lang::En => format!(
                "1τ: {complete_1:.3}% complete/{remaining_1:.3}% error · 3τ: {complete_3:.3}%/{remaining_3:.3}% · 5τ: {complete_5:.3}%/{remaining_5:.3}%"
            ),
        };
        formula_box(
            ui,
            t,
            &[
                "τ = R_th · C",
                "Vc(t) = V_final + (V_initial − V_final) · exp(−t/τ)",
                text(
                    lang,
                    "充电特例: V_initial=0, V_final=Vs  ·  放电特例: V_initial=V0, V_final=0",
                    "Charge: V_initial=0, V_final=Vs  ·  Discharge: V_initial=V0, V_final=0",
                ),
                &settling_note,
                text(
                    lang,
                    "R_th 是独立源置零后从电容端看入的总等效电阻，可含源/负载/ESR。本模型限于线性单主导极点；容差、漏电和 ESR 会引入偏差。",
                    "R_th is the total resistance seen by C with independent sources zeroed, including source/load/ESR as needed. Linear single-pole model; tolerance, leakage and ESR cause error.",
                ),
            ],
        );
    }

    fn crc_ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        tool_header(
            ui,
            t,
            5,
            text(lang, "CRC 计算器", "CRC Calculator"),
            text(
                lang,
                "常用预设与自定义多项式",
                "Common presets and custom polynomials",
            ),
        );
        section_heading(ui, t, text(lang, "输入", "Input"));
        ui.horizontal(|ui| {
            ui.add_sized(
                [82.0, 24.0],
                egui::Label::new(text(lang, "数据模式", "Data mode")),
            );
            ui.selectable_value(&mut self.crc.input_mode, CrcInputMode::Hex, "HEX");
            ui.selectable_value(
                &mut self.crc.input_mode,
                CrcInputMode::Text,
                text(lang, "文本", "UTF-8"),
            );
        });
        ui.add_sized(
            [ui.available_width(), 54.0],
            egui::TextEdit::multiline(&mut self.crc.data).hint_text(match self.crc.input_mode {
                CrcInputMode::Hex => "01 02, 0xA5, FF",
                CrcInputMode::Text => "123456789",
            }),
        );

        section_heading(ui, t, text(lang, "参数", "Parameters"));
        ui.horizontal(|ui| {
            ui.add_sized(
                [82.0, 24.0],
                egui::Label::new(text(lang, "CRC 类型", "CRC type")),
            );
            egui::ComboBox::from_id_salt("crc_preset")
                .width((ui.available_width() - 4.0).max(120.0))
                .selected_text(self.crc.preset.name())
                .show_ui(ui, |ui| {
                    for preset in CrcPreset::ALL {
                        ui.selectable_value(&mut self.crc.preset, preset, preset.name());
                    }
                });
        });
        if self.crc.preset == CrcPreset::Custom {
            ui.horizontal(|ui| {
                ui.add_sized([82.0, 24.0], egui::Label::new("width"));
                egui::ComboBox::from_id_salt("crc_custom_width")
                    .width(70.0)
                    .selected_text(self.crc.custom_width.to_string())
                    .show_ui(ui, |ui| {
                        for width in [8, 16, 32] {
                            ui.selectable_value(
                                &mut self.crc.custom_width,
                                width,
                                width.to_string(),
                            );
                        }
                    });
            });
            plain_value_row(ui, "poly", &mut self.crc.custom_poly, "hex");
            plain_value_row(ui, "init", &mut self.crc.custom_init, "hex");
            plain_value_row(ui, "xorout", &mut self.crc.custom_xorout, "hex");
            ui.horizontal(|ui| {
                ui.add_sized([82.0, 24.0], egui::Label::new("reflection"));
                ui.checkbox(&mut self.crc.custom_refin, "refin");
                ui.checkbox(&mut self.crc.custom_refout, "refout");
            });
        }
        if let Ok(params) = self.crc.active_params() {
            result_table(
                ui,
                t,
                &[
                    ("width", params.width.to_string()),
                    ("poly", format_crc_hex(params.poly, params.width)),
                    ("init", format_crc_hex(params.init, params.width)),
                    (
                        "refin / refout",
                        format!("{} / {}", params.refin, params.refout),
                    ),
                    ("xorout", format_crc_hex(params.xorout, params.width)),
                ],
            );
        }
        if ui_theme::accent_button(ui, t, text(lang, "计算", "Calculate")).clicked() {
            self.crc.calculate();
        }
        error_slot(ui, self.crc.error.map(|error| crc_error_text(lang, error)));

        section_heading(ui, t, text(lang, "结果", "Results"));
        let result = self
            .crc
            .result
            .and_then(|value| {
                self.crc
                    .active_params()
                    .ok()
                    .map(|params| format_crc_hex(value, params.width))
            })
            .unwrap_or_else(|| "—".into());
        result_table(ui, t, &[("CRC", result)]);

        section_heading(ui, t, text(lang, "公式与说明", "Formula & notes"));
        formula_box(
            ui,
            t,
            &[
                text(
                    lang,
                    "逐位：若寄存器最高位为 1，则 (crc<<1) XOR poly；反射算法从最低位右移。",
                    "Bitwise: MSB=1 uses (crc<<1) XOR poly; reflected CRC shifts from the LSB.",
                ),
                text(
                    lang,
                    "poly 输入使用正常（非反射）表示；内部会按 refin 自动反射。",
                    "Enter poly in normal form; it is reflected internally when refin=true.",
                ),
            ],
        );
    }
}

fn rc_error_text(lang: Lang, error: &str) -> &'static str {
    match (lang, error) {
        (Lang::Zh, "resistance") => "等效电阻 R_th 必须是有限正值。",
        (Lang::En, "resistance") => "Equivalent resistance R_th must be finite and positive.",
        (Lang::Zh, "capacitance") => "电容 C 必须是有限正值。",
        (Lang::En, "capacitance") => "Capacitance C must be a finite positive value.",
        (Lang::Zh, "voltage") => "初始/最终电压必须是有限值（可为负）。",
        (Lang::En, "voltage") => "Initial/final voltages must be finite (negative is allowed).",
        (Lang::Zh, "time") => "观察时间 t 必须是有限非负值。",
        (Lang::En, "time") => "Time t must be finite and non-negative.",
        (Lang::Zh, "time_constant") => "R_th·C 发生溢出或下溢，请调整输入。",
        (Lang::En, "time_constant") => "R_th·C overflowed or underflowed; adjust the inputs.",
        (Lang::Zh, _) => "参数导致结果非有限，请调整输入。",
        (Lang::En, _) => "The result is not finite; adjust the inputs.",
    }
}

fn crc_error_text(lang: Lang, error: CrcError) -> &'static str {
    match (lang, error) {
        (Lang::Zh, CrcError::InvalidHexData) => {
            "HEX 数据无效；请使用成对十六进制字节，可用空格、逗号或 0x 前缀。"
        }
        (Lang::En, CrcError::InvalidHexData) => {
            "Invalid HEX data; use byte pairs separated by spaces/commas, optionally with 0x."
        }
        (Lang::Zh, CrcError::InvalidWidth) => "位宽必须为 8、16 或 32。",
        (Lang::En, CrcError::InvalidWidth) => "Width must be 8, 16, or 32.",
        (Lang::Zh, CrcError::InvalidPoly) => "poly 必须是位宽范围内的非零十六进制值。",
        (Lang::En, CrcError::InvalidPoly) => {
            "poly must be a non-zero hexadecimal value within the selected width."
        }
        (Lang::Zh, CrcError::InvalidInit) => "init 十六进制值超出所选位宽。",
        (Lang::En, CrcError::InvalidInit) => "init exceeds the selected CRC width.",
        (Lang::Zh, CrcError::InvalidXorout) => "xorout 十六进制值超出所选位宽。",
        (Lang::En, CrcError::InvalidXorout) => "xorout exceeds the selected CRC width.",
    }
}

fn format_crc_hex(value: u32, width: u8) -> String {
    format!("0x{value:0digits$X}", digits = (width / 4) as usize)
}

fn text<'a>(lang: Lang, zh: &'a str, en: &'a str) -> &'a str {
    match lang {
        Lang::Zh => zh,
        Lang::En => en,
    }
}

fn tool_header(ui: &mut egui::Ui, t: &Tokens, number: usize, title: &str, description: &str) {
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.heading(
        egui::RichText::new(format!("{number}. {title}"))
            .size(16.0)
            .color(t.text_primary),
    );
    ui.label(egui::RichText::new(description).small().color(t.text_muted));
    ui.add_space(2.0);
}

fn section_heading(ui: &mut egui::Ui, t: &Tokens, title: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(title)
            .size(12.0)
            .strong()
            .color(t.text_primary),
    );
    ui.separator();
}

fn placeholder_ui(ui: &mut egui::Ui, lang: Lang, t: &Tokens, number: usize) {
    tool_header(
        ui,
        t,
        number,
        text(lang, "预留工具", "Reserved Tool"),
        text(
            lang,
            "功能将在后续版本提供",
            "Functionality planned for a future release",
        ),
    );
    section_heading(ui, t, text(lang, "内容", "Content"));
    formula_box(
        ui,
        t,
        &[text(lang, "预留 / 即将推出", "Reserved / Coming soon")],
    );
}

fn plain_value_row(ui: &mut egui::Ui, label: &str, value: &mut String, suffix: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([82.0, 24.0], egui::Label::new(label));
        ui.add_sized(
            [(ui.available_width() - 68.0).max(64.0), 24.0],
            egui::TextEdit::singleline(value),
        );
        ui.add_sized(
            [58.0, 24.0],
            egui::Label::new(egui::RichText::new(suffix).small()),
        );
    });
}

#[derive(Debug, Clone, Copy)]
struct InputGridMetrics {
    label_width: f32,
    value_width: f32,
    unit_width: f32,
    gap: f32,
}

fn input_grid_metrics(available_width: f32) -> InputGridMetrics {
    let gap = 5.0;
    let unit_width = (available_width * 0.25).clamp(50.0, 58.0);
    let label_width = (available_width * 0.31).clamp(58.0, 82.0);
    let value_width = (available_width - label_width - unit_width - gap * 2.0).max(58.0);
    InputGridMetrics {
        label_width,
        value_width,
        unit_width,
        gap,
    }
}

fn compact_grid_row<U: Copy + PartialEq>(
    ui: &mut egui::Ui,
    metrics: InputGridMetrics,
    label: &str,
    value: &mut String,
    selected: &mut U,
    units: &[U],
    unit_label: impl Fn(U) -> &'static str,
    id: &'static str,
) {
    ui.add_sized([metrics.label_width, 26.0], egui::Label::new(label));
    ui.add_sized(
        [metrics.value_width, 26.0],
        egui::TextEdit::singleline(value),
    );
    egui::ComboBox::from_id_salt(id)
        .width(metrics.unit_width)
        .selected_text(unit_label(*selected))
        .show_ui(ui, |ui| {
            for unit in units {
                ui.selectable_value(selected, *unit, unit_label(*unit));
            }
        });
    ui.end_row();
}

fn compact_plain_grid_row(
    ui: &mut egui::Ui,
    metrics: InputGridMetrics,
    label: &str,
    value: &mut String,
    suffix: &str,
) {
    ui.add_sized([metrics.label_width, 26.0], egui::Label::new(label));
    ui.add_sized(
        [metrics.value_width, 26.0],
        egui::TextEdit::singleline(value),
    );
    ui.add_sized(
        [metrics.unit_width, 26.0],
        egui::Label::new(egui::RichText::new(suffix).small()),
    );
    ui.end_row();
}

fn compact_calculate_button(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: &str,
    metrics: InputGridMetrics,
) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = metrics.gap;
        ui.add_space(metrics.label_width);
        ui.add_sized(
            [metrics.value_width + metrics.unit_width + metrics.gap, 30.0],
            egui::Button::new(egui::RichText::new(label).color(t.accent_text).strong())
                .fill(t.accent)
                .stroke(Stroke::NONE)
                .corner_radius(CornerRadius::same(5)),
        )
        .clicked()
    })
    .inner
}

fn result_table(ui: &mut egui::Ui, t: &Tokens, rows: &[(&str, String)]) {
    Frame::NONE
        .fill(t.surface_bg)
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            egui::Grid::new(ui.next_auto_id())
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    for (name, value) in rows {
                        ui.add_sized(
                            [58.0, 18.0],
                            egui::Label::new(egui::RichText::new(*name).color(t.text_muted)),
                        );
                        ui.label(egui::RichText::new(value).strong().color(t.accent));
                        ui.end_row();
                    }
                });
        });
}

fn error_slot(ui: &mut egui::Ui, error: Option<&str>) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 34.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            if let Some(error) = error {
                ui.label(
                    egui::RichText::new(error)
                        .small()
                        .color(Color32::from_rgb(239, 68, 68)),
                );
            }
        },
    );
}

fn formula_box(ui: &mut egui::Ui, t: &Tokens, lines: &[&str]) {
    Frame::NONE
        .fill(t.surface_bg)
        .stroke(Stroke::new(1.0_f32, t.border.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for line in lines {
                ui.label(egui::RichText::new(*line).small().color(t.text_muted));
            }
        });
}

#[derive(Debug, Clone, Copy)]
struct BandpassSchematicGeometry {
    vin: Pos2,
    c1_left: Pos2,
    c1_right: Pos2,
    high_pass_node: Pos2,
    buffer_in: Pos2,
    buffer_out: Pos2,
    r2_left: Pos2,
    r2_right: Pos2,
    vout: Pos2,
    ground_y: f32,
}

fn bandpass_schematic_geometry(rect: Rect) -> BandpassSchematicGeometry {
    let x = |fraction: f32| rect.left() + rect.width() * fraction;
    let signal_y = rect.top() + rect.height() * 0.38;
    BandpassSchematicGeometry {
        vin: Pos2::new(x(0.04), signal_y),
        c1_left: Pos2::new(x(0.16), signal_y),
        c1_right: Pos2::new(x(0.21), signal_y),
        high_pass_node: Pos2::new(x(0.29), signal_y),
        buffer_in: Pos2::new(x(0.37), signal_y),
        buffer_out: Pos2::new(x(0.52), signal_y),
        r2_left: Pos2::new(x(0.57), signal_y),
        r2_right: Pos2::new(x(0.73), signal_y),
        vout: Pos2::new(x(0.84), signal_y),
        ground_y: rect.top() + rect.height() * 0.86,
    }
}

fn bandpass_schematic(ui: &mut egui::Ui, t: &Tokens, lang: Lang) {
    let width = ui.available_width().max(180.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 170.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(4), t.surface_bg);
    let drawing = rect.shrink(6.0);
    let g = bandpass_schematic_geometry(drawing);
    let stroke = Stroke::new(1.5_f32, t.text_primary);
    let accent_stroke = Stroke::new(1.5_f32, t.accent);
    let painter = ui.painter();

    painter.text(
        Pos2::new((g.vin.x + g.high_pass_node.x) * 0.5, drawing.top() + 2.0),
        egui::Align2::CENTER_TOP,
        text(lang, "高通级", "High-pass"),
        egui::FontId::proportional(10.0),
        t.text_muted,
    );
    painter.text(
        Pos2::new((g.r2_left.x + g.vout.x) * 0.5, drawing.top() + 2.0),
        egui::Align2::CENTER_TOP,
        text(lang, "低通级", "Low-pass"),
        egui::FontId::proportional(10.0),
        t.text_muted,
    );

    painter.line_segment([g.vin, g.c1_left], stroke);
    let plate_half = 10.0;
    painter.line_segment(
        [
            Pos2::new(g.c1_left.x, g.c1_left.y - plate_half),
            Pos2::new(g.c1_left.x, g.c1_left.y + plate_half),
        ],
        accent_stroke,
    );
    painter.line_segment(
        [
            Pos2::new(g.c1_right.x, g.c1_right.y - plate_half),
            Pos2::new(g.c1_right.x, g.c1_right.y + plate_half),
        ],
        accent_stroke,
    );
    painter.line_segment([g.c1_right, g.high_pass_node], stroke);
    painter.circle_filled(g.high_pass_node, 2.5, t.accent);
    painter.text(
        Pos2::new((g.c1_left.x + g.c1_right.x) * 0.5, g.c1_left.y - 14.0),
        egui::Align2::CENTER_BOTTOM,
        "C1",
        egui::FontId::proportional(10.0),
        t.text_primary,
    );

    painter.line_segment(
        [
            g.high_pass_node,
            Pos2::new(g.high_pass_node.x, g.high_pass_node.y + 12.0),
        ],
        stroke,
    );
    draw_vertical_resistor(
        painter,
        g.high_pass_node.x,
        g.high_pass_node.y + 12.0,
        g.ground_y - 16.0,
        stroke,
    );
    draw_ground(painter, g.high_pass_node.x, g.ground_y, stroke);
    painter.text(
        Pos2::new(
            g.high_pass_node.x + 8.0,
            (g.high_pass_node.y + g.ground_y) * 0.5,
        ),
        egui::Align2::LEFT_CENTER,
        "R1",
        egui::FontId::proportional(10.0),
        t.text_primary,
    );

    painter.line_segment([g.high_pass_node, g.buffer_in], stroke);
    let triangle = vec![
        Pos2::new(g.buffer_in.x, g.buffer_in.y - 16.0),
        Pos2::new(g.buffer_in.x, g.buffer_in.y + 16.0),
        g.buffer_out,
        Pos2::new(g.buffer_in.x, g.buffer_in.y - 16.0),
    ];
    painter.add(egui::Shape::line(triangle, accent_stroke));
    painter.text(
        Pos2::new((g.buffer_in.x + g.buffer_out.x) * 0.5, g.buffer_in.y + 20.0),
        egui::Align2::CENTER_TOP,
        text(lang, "理想缓冲", "Ideal buffer"),
        egui::FontId::proportional(9.0),
        t.accent,
    );

    painter.line_segment([g.buffer_out, g.r2_left], stroke);
    draw_horizontal_resistor(painter, g.r2_left, g.r2_right, stroke);
    painter.line_segment([g.r2_right, g.vout], stroke);
    painter.circle_filled(g.vout, 2.5, t.accent);
    painter.text(
        Pos2::new((g.r2_left.x + g.r2_right.x) * 0.5, g.r2_left.y - 14.0),
        egui::Align2::CENTER_BOTTOM,
        "R2",
        egui::FontId::proportional(10.0),
        t.text_primary,
    );

    let c2_top = g.vout.y + 22.0;
    let c2_bottom = c2_top + 8.0;
    painter.line_segment([g.vout, Pos2::new(g.vout.x, c2_top)], stroke);
    painter.line_segment(
        [
            Pos2::new(g.vout.x - 10.0, c2_top),
            Pos2::new(g.vout.x + 10.0, c2_top),
        ],
        accent_stroke,
    );
    painter.line_segment(
        [
            Pos2::new(g.vout.x - 10.0, c2_bottom),
            Pos2::new(g.vout.x + 10.0, c2_bottom),
        ],
        accent_stroke,
    );
    painter.line_segment(
        [
            Pos2::new(g.vout.x, c2_bottom),
            Pos2::new(g.vout.x, g.ground_y - 6.0),
        ],
        stroke,
    );
    draw_ground(painter, g.vout.x, g.ground_y, stroke);
    painter.text(
        Pos2::new(g.vout.x + 12.0, (c2_bottom + g.ground_y) * 0.5),
        egui::Align2::LEFT_CENTER,
        "C2",
        egui::FontId::proportional(10.0),
        t.text_primary,
    );

    painter.text(
        Pos2::new(g.vin.x, g.vin.y + 12.0),
        egui::Align2::CENTER_TOP,
        "Vin",
        egui::FontId::proportional(10.0),
        t.text_primary,
    );
    painter.text(
        Pos2::new(g.vout.x, g.vout.y + 12.0),
        egui::Align2::CENTER_TOP,
        "Vout",
        egui::FontId::proportional(10.0),
        t.text_primary,
    );
    painter.text(
        Pos2::new((g.high_pass_node.x + g.vout.x) * 0.5, g.ground_y + 3.0),
        egui::Align2::CENTER_TOP,
        "GND",
        egui::FontId::proportional(9.0),
        t.text_muted,
    );
}

fn draw_horizontal_resistor(painter: &egui::Painter, start: Pos2, end: Pos2, stroke: Stroke) {
    let step = (end.x - start.x) / 8.0;
    let mut points = Vec::with_capacity(9);
    for index in 0..=8 {
        let y = if index == 0 || index == 8 {
            start.y
        } else if index % 2 == 0 {
            start.y - 5.0
        } else {
            start.y + 5.0
        };
        points.push(Pos2::new(start.x + step * index as f32, y));
    }
    painter.add(egui::Shape::line(points, stroke));
}

fn draw_vertical_resistor(painter: &egui::Painter, x: f32, top: f32, bottom: f32, stroke: Stroke) {
    let step = (bottom - top) / 8.0;
    let mut points = Vec::with_capacity(9);
    for index in 0..=8 {
        let point_x = if index == 0 || index == 8 {
            x
        } else if index % 2 == 0 {
            x - 5.0
        } else {
            x + 5.0
        };
        points.push(Pos2::new(point_x, top + step * index as f32));
    }
    painter.add(egui::Shape::line(points, stroke));
}

fn draw_ground(painter: &egui::Painter, x: f32, y: f32, stroke: Stroke) {
    painter.line_segment([Pos2::new(x, y - 6.0), Pos2::new(x, y - 1.0)], stroke);
    for (offset, half_width) in [(0.0, 8.0), (3.0, 5.0), (6.0, 2.0)] {
        painter.line_segment(
            [
                Pos2::new(x - half_width, y + offset),
                Pos2::new(x + half_width, y + offset),
            ],
            stroke,
        );
    }
}

fn current_plot(ui: &mut egui::Ui, t: &Tokens, curve: &[[f64; 2]]) {
    let width = ui.available_width().max(80.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 180.0), Sense::hover());
    let plot = rect.shrink2(Vec2::new(42.0, 26.0));
    ui.painter()
        .rect_filled(rect, CornerRadius::same(4), t.surface_bg);
    ui.painter().rect_stroke(
        plot,
        CornerRadius::ZERO,
        Stroke::new(1.0_f32, t.border),
        egui::StrokeKind::Inside,
    );
    if curve.len() < 2 {
        return;
    }
    let min_frequency = curve.first().map(|point| point[0]).unwrap_or(0.0);
    let max_frequency = curve.last().map(|point| point[0]).unwrap_or(1.0);
    let frequency_span = (max_frequency - min_frequency).max(f64::EPSILON);
    let tick_count = 9;
    let show_every = if plot.width() >= 360.0 { 1 } else { 2 };
    for index in 0..tick_count {
        let fraction = index as f32 / (tick_count - 1) as f32;
        let x = plot.left() + plot.width() * fraction;
        ui.painter().line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(0.7_f32, t.border.gamma_multiply(0.45)),
        );
        if index % show_every == 0 {
            let frequency = min_frequency + frequency_span * f64::from(fraction);
            ui.painter().text(
                Pos2::new(x, plot.bottom() + 4.0),
                egui::Align2::CENTER_TOP,
                format_axis_frequency(frequency),
                egui::FontId::proportional(9.0),
                t.text_muted,
            );
        }
    }
    let max_current = curve
        .iter()
        .map(|point| point[1])
        .filter(|value| value.is_finite())
        .fold(0.0_f64, f64::max)
        .max(f64::EPSILON);
    let points: Vec<Pos2> = curve
        .iter()
        .filter_map(|point| {
            if !point[0].is_finite() || point[0] <= 0.0 || !point[1].is_finite() {
                return None;
            }
            let x =
                plot.left() + ((point[0] - min_frequency) / frequency_span) as f32 * plot.width();
            let y = plot.bottom() - (point[1] / max_current) as f32 * plot.height();
            Some(Pos2::new(x, y))
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(1.8_f32, t.accent)));
    ui.painter().text(
        Pos2::new(plot.left() - 3.0, plot.top()),
        egui::Align2::RIGHT_TOP,
        format!("{max_current:.3} A"),
        egui::FontId::proportional(9.0),
        t.text_muted,
    );
    ui.painter().text(
        Pos2::new(plot.left() - 3.0, plot.bottom()),
        egui::Align2::RIGHT_BOTTOM,
        "0 A",
        egui::FontId::proportional(9.0),
        t.text_muted,
    );
}

fn format_axis_frequency(hz: f64) -> String {
    if hz >= 1.0e6 {
        format!("{:.2}M", hz / 1.0e6)
    } else {
        format!("{:.0}k", hz / 1.0e3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_number_accepts_hex_literals() {
        assert_eq!(parse_number("0x10", CalcError::InvalidInductance).unwrap(), 16.0);
        assert_eq!(parse_number("FFH", CalcError::InvalidInductance).unwrap(), 255.0);
        assert_eq!(parse_number("10", CalcError::InvalidInductance).unwrap(), 10.0);
    }

    #[test]
    fn converts_supported_units() {
        assert!(
            (inductance_h(10.0, InductanceUnit::Microhenry).unwrap() - 10.0e-6).abs() < 1.0e-18
        );
        assert!(
            (capacitance_f(100.0, CapacitanceUnit::Nanofarad).unwrap() - 100.0e-9).abs() < 1.0e-18
        );
        assert_eq!(
            resistance_ohm(2.0, ResistanceUnit::Kiloohm, false).unwrap(),
            2000.0
        );
        assert_eq!(
            resistance_ohm(3.0, ResistanceUnit::Megaohm, false).unwrap(),
            3.0e6
        );
        assert_eq!(
            capacitance_f(4.0, CapacitanceUnit::Picofarad).unwrap(),
            4.0e-12
        );
    }

    #[test]
    fn input_grid_uses_available_half_column_width() {
        for width in [190.0_f32, 300.0, 420.0] {
            let metrics = input_grid_metrics(width);
            let used =
                metrics.label_width + metrics.value_width + metrics.unit_width + 2.0 * metrics.gap;
            assert!((used - width).abs() < 0.01);
            assert!(metrics.value_width >= 58.0);
            assert!(metrics.unit_width >= 50.0);
        }
        assert!(input_grid_metrics(300.0).value_width > input_grid_metrics(190.0).value_width);
    }

    #[test]
    fn computes_lc_resonance() {
        let frequency = resonance_frequency(10.0e-6, 100.0e-9).unwrap();
        assert!((frequency - 159_154.943).abs() < 0.01);
    }

    #[test]
    fn resonance_curve_is_finite_and_peaks_near_f0() {
        let l = 10.0e-6;
        let c = 100.0e-9;
        let f0 = resonance_frequency(l, c).unwrap();
        let curve = resonance_curve(f0, l, c, 0.0);
        assert!(curve.iter().all(|point| point[1].is_finite()));
        let peak = curve
            .iter()
            .max_by(|left, right| left[1].total_cmp(&right[1]))
            .unwrap();
        assert!((peak[0] / f0 - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn resonance_curve_uses_linear_plus_minus_200_khz_bounds() {
        let (low, high) = resonance_frequency_bounds(500_000.0);
        assert_eq!(low, 300_000.0);
        assert_eq!(high, 700_000.0);
        let curve = resonance_curve(500_000.0, 10.0e-6, 10.0e-9, 0.1);
        assert_eq!(curve.first().unwrap()[0], low);
        assert_eq!(curve.last().unwrap()[0], high);
        assert!(curve.iter().any(|point| point[0] == 500_000.0));
        assert!(curve.iter().all(|point| point[1].is_finite()));
    }

    #[test]
    fn resonance_curve_clamps_low_frequency_and_includes_f0() {
        let f0 = 100_000.0;
        let (low, high) = resonance_frequency_bounds(f0);
        assert_eq!(low, MIN_PLOT_FREQUENCY_HZ);
        assert_eq!(high, 300_000.0);
        let curve = resonance_curve(f0, 10.0e-6, 100.0e-9, 0.0);
        assert_eq!(curve.first().unwrap()[0], MIN_PLOT_FREQUENCY_HZ);
        assert!(curve.iter().any(|point| point[0] == f0));
        assert!(curve.iter().all(|point| point[1].is_finite()));
    }

    #[test]
    fn rejects_invalid_lc_inputs() {
        assert_eq!(
            inductance_h(0.0, InductanceUnit::Henry),
            Err(CalcError::InvalidInductance)
        );
        assert_eq!(
            capacitance_f(f64::NAN, CapacitanceUnit::Farad),
            Err(CalcError::InvalidCapacitance)
        );
        assert_eq!(
            resistance_ohm(-1.0, ResistanceUnit::Ohm, true),
            Err(CalcError::InvalidResistance)
        );
        assert_eq!(resistance_ohm(0.0, ResistanceUnit::Ohm, true).unwrap(), 0.0);
    }

    #[test]
    fn computes_valid_bandpass() {
        let result = bandpass_result(10_000.0, 100.0e-9, 10_000.0, 1.0e-9).unwrap();
        assert!(result.f_low_hz < result.f_high_hz);
        assert!(result.bandwidth_hz > 0.0);
        assert!((result.center_hz - (result.f_low_hz * result.f_high_hz).sqrt()).abs() < 1.0e-9);
        assert!(result.q.is_finite());
    }

    #[test]
    fn rejects_invalid_bandpass() {
        assert_eq!(
            bandpass_result(10_000.0, 1.0e-9, 10_000.0, 100.0e-9).unwrap_err(),
            CalcError::NoPassband
        );
        assert_eq!(
            bandpass_result(0.0, 1.0e-9, 10_000.0, 100.0e-9).unwrap_err(),
            CalcError::InvalidHighPass
        );
    }

    #[test]
    fn computes_logarithmic_decrement_q() {
        let result = logarithmic_decrement_q(10.0, 8.0, 0.0, 2.0).unwrap();
        assert!((result.decrement - (1.25_f64.ln() / 2.0)).abs() < 1.0e-12);
        let expected_exact =
            (4.0 * PI * PI + result.decrement * result.decrement).sqrt() / (2.0 * result.decrement);
        assert!((result.exact_q - expected_exact).abs() < 1.0e-12);
        assert!((result.approx_q - PI / result.decrement).abs() < 1.0e-12);
        assert!((result.damping_ratio - 1.0 / (2.0 * result.exact_q)).abs() < 1.0e-12);

        let with_bias = logarithmic_decrement_q(12.0, 10.0, 2.0, 2.0).unwrap();
        assert!((with_bias.decrement - result.decrement).abs() < 1.0e-12);
        assert!((with_bias.exact_q - result.exact_q).abs() < 1.0e-12);
    }

    #[test]
    fn q_exact_and_approx_behave_across_damping_ranges() {
        let high_q = logarithmic_decrement_q(0.01_f64.exp(), 1.0, 0.0, 1.0).unwrap();
        assert!((high_q.exact_q - high_q.approx_q) / high_q.exact_q < 1.0e-5);

        let larger_delta = logarithmic_decrement_q(2.0_f64.exp(), 1.0, 0.0, 1.0).unwrap();
        assert!((larger_delta.exact_q - larger_delta.approx_q).abs() > 0.05);

        let result = logarithmic_decrement_q(PI.exp(), 1.0, 0.0, 1.0).unwrap();
        assert!((result.decrement - PI).abs() < 1.0e-12);
        assert!((result.exact_q - 5.0_f64.sqrt() / 2.0).abs() < 1.0e-12);
        assert!((result.approx_q - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn q_formula_text_contains_exact_and_approx_terms() {
        assert!(Q_DAMPING_FORMULA.contains("4π²"));
        assert!(Q_EXACT_FORMULA.contains("4π²"));
        assert!(Q_EXACT_FORMULA.contains("2δ"));
        assert!(Q_APPROX_FORMULA.contains("πN"));
        assert!(Q_DECREMENT_FORMULA.contains("(V1−Bias)/(V2−Bias)"));
        assert!(Q_APPROX_FORMULA.contains("(V1−Bias)/(V2−Bias)"));
    }

    #[test]
    fn rejects_invalid_q_inputs() {
        assert_eq!(
            logarithmic_decrement_q(8.0, 8.0, 0.0, 1.0).unwrap_err(),
            CalcError::InvalidPeaks
        );
        assert_eq!(
            logarithmic_decrement_q(1.0, 0.0, 0.0, 1.0).unwrap_err(),
            CalcError::InvalidPeaks
        );
        assert_eq!(
            logarithmic_decrement_q(10.0, 8.0, 8.0, 1.0).unwrap_err(),
            CalcError::InvalidPeaks
        );
        assert_eq!(
            logarithmic_decrement_q(10.0, 8.0, 0.0, 0.0).unwrap_err(),
            CalcError::InvalidInterval
        );
        assert_eq!(
            logarithmic_decrement_q(10.0, 8.0, 0.0, 1.5).unwrap_err(),
            CalcError::InvalidInterval
        );
    }

    #[test]
    fn converts_rc_units_and_computes_response() {
        assert_eq!(
            resistance_ohm(2.0, ResistanceUnit::Milliohm, false).unwrap(),
            0.002
        );
        assert_eq!(time_seconds(250.0, TimeUnit::Microsecond).unwrap(), 0.00025);
        let result = rc_time_response(10_000.0, 100.0e-6, 0.0, 5.0, 1.0).unwrap();
        assert!((result.tau_s - 1.0).abs() < 1.0e-12);
        assert!((result.capacitor_voltage_v - 5.0 * (1.0 - (-1.0_f64).exp())).abs() < 1.0e-12);
    }

    #[test]
    fn rc_step_response_handles_initial_final_and_negative_voltages() {
        let at_zero = rc_time_response(1.0, 1.0, -3.0, 7.0, 0.0).unwrap();
        assert_eq!(at_zero.capacitor_voltage_v, -3.0);

        let charging = rc_time_response(1.0, 1.0, 0.0, 5.0, 1.0).unwrap();
        assert!((charging.capacitor_voltage_v - 5.0 * (1.0 - (-1.0_f64).exp())).abs() < 1e-12);

        let discharging = rc_time_response(1.0, 1.0, 5.0, 0.0, 1.0).unwrap();
        assert!((discharging.capacitor_voltage_v - 5.0 * (-1.0_f64).exp()).abs() < 1e-12);

        let negative_step = rc_time_response(1.0, 1.0, -2.0, -6.0, 1.0).unwrap();
        let expected = -6.0 + 4.0 * (-1.0_f64).exp();
        assert!((negative_step.capacitor_voltage_v - expected).abs() < 1e-12);

        let settled = rc_time_response(1.0, 1.0, 2.0, -4.0, 1.0e300).unwrap();
        assert_eq!(settled.capacitor_voltage_v, -4.0);
    }

    #[test]
    fn rc_settling_percentages_match_standard_values() {
        for (multiple, complete_expected, remaining_expected) in [
            (1.0, 63.212, 36.788),
            (3.0, 95.021, 4.979),
            (5.0, 99.326, 0.674),
        ] {
            let (complete, remaining) = rc_settling_percentages(multiple);
            assert!((complete - complete_expected).abs() < 0.001);
            assert!((remaining - remaining_expected).abs() < 0.001);
        }
    }

    #[test]
    fn rejects_invalid_rc_inputs() {
        assert_eq!(
            rc_time_response(0.0, 1.0e-6, 0.0, 5.0, 1.0),
            Err("resistance")
        );
        assert_eq!(
            rc_time_response(1.0, -1.0, 0.0, 5.0, 1.0),
            Err("capacitance")
        );
        assert_eq!(
            rc_time_response(1.0, 1.0, f64::NAN, 1.0, 1.0),
            Err("voltage")
        );
        assert_eq!(
            rc_time_response(1.0, 1.0, 0.0, f64::INFINITY, 1.0),
            Err("voltage")
        );
        assert_eq!(rc_time_response(1.0, 1.0, 0.0, 5.0, -1.0), Err("time"));
        assert_eq!(
            rc_time_response(f64::MAX, f64::MAX, 0.0, 1.0, 1.0),
            Err("time_constant")
        );
        assert_eq!(
            rc_time_response(f64::MIN_POSITIVE, f64::MIN_POSITIVE, 0.0, 1.0, 1.0),
            Err("time_constant")
        );
    }

    #[test]
    fn crc_presets_match_standard_check_values() {
        let data = b"123456789";
        let checks = [
            (CrcPreset::Crc8, 0xF4),
            (CrcPreset::Crc8Maxim, 0xA1),
            (CrcPreset::Crc16Arc, 0xBB3D),
            (CrcPreset::Crc16Modbus, 0x4B37),
            (CrcPreset::Crc16CcittFalse, 0x29B1),
            (CrcPreset::Crc16Xmodem, 0x31C3),
            (CrcPreset::Crc32IsoHdlc, 0xCBF4_3926),
            (CrcPreset::Crc32C, 0xE306_9283),
        ];
        for (preset, expected) in checks {
            assert_eq!(
                calculate_crc(data, preset.params().unwrap()).unwrap(),
                expected,
                "{}",
                preset.name()
            );
        }
    }

    #[test]
    fn parses_flexible_hex_data() {
        assert_eq!(
            parse_hex_data("01 02, 0xA5; FF 0x1 1234").unwrap(),
            vec![0x01, 0x02, 0xA5, 0xFF, 0x01, 0x12, 0x34]
        );
        assert_eq!(parse_hex_data("123").unwrap_err(), CrcError::InvalidHexData);
        assert_eq!(parse_hex_data("GG").unwrap_err(), CrcError::InvalidHexData);
    }

    #[test]
    fn rejects_invalid_custom_crc_parameters() {
        assert_eq!(
            validate_crc_params(CrcParams {
                width: 12,
                poly: 0x80F,
                init: 0,
                refin: false,
                refout: false,
                xorout: 0,
            })
            .unwrap_err(),
            CrcError::InvalidWidth
        );
        assert_eq!(
            parse_crc_hex("0x1FF", 8, CrcError::InvalidPoly).unwrap_err(),
            CrcError::InvalidPoly
        );
        assert_eq!(
            validate_crc_params(CrcParams {
                width: 8,
                poly: 0,
                init: 0,
                refin: false,
                refout: false,
                xorout: 0,
            })
            .unwrap_err(),
            CrcError::InvalidPoly
        );
    }

    #[test]
    fn bandpass_schematic_nodes_are_finite_and_ordered() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(300.0, 160.0));
        let geometry = bandpass_schematic_geometry(rect);
        let nodes = [
            geometry.vin,
            geometry.c1_left,
            geometry.c1_right,
            geometry.high_pass_node,
            geometry.buffer_in,
            geometry.buffer_out,
            geometry.r2_left,
            geometry.r2_right,
            geometry.vout,
        ];
        assert!(nodes
            .iter()
            .all(|point| point.x.is_finite() && point.y.is_finite()));
        assert!(geometry.ground_y.is_finite());
        assert!(nodes.windows(2).all(|pair| pair[0].x < pair[1].x));
        assert!(geometry.ground_y > geometry.vin.y);
        assert!(nodes.iter().all(|point| rect.contains(*point)));
    }
}
