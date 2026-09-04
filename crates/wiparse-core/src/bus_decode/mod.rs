//! Software bus decode from analog scope traces (offline waveform analysis).

mod ddsss;
mod digital;
mod i2c;
mod i2s;
mod spi;
mod uart;

pub use ddsss::{
    decode_ddsss, qi_ask_frame, synthesize_ddsss, DdsssConfig, DdsssExtension, DdsssSequence,
    DdsssSynthRequest,
};
pub use digital::{analog_to_edges, default_threshold, DigitalEdge, EdgeKind};
pub use i2c::{decode_i2c, I2cConfig};
pub use i2s::{decode_i2s, I2sConfig, I2sFormat};
pub use spi::{decode_spi, SpiConfig, SpiMode, SpiWire};
pub use uart::{decode_uart, UartConfig, UartParity};

use crate::instrument::WaveformTrace;

/// Supported bus protocols for waveform analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BusKind {
    #[default]
    Off,
    Uart,
    I2c,
    Spi,
    I2s,
    Ddsss,
}

impl BusKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Uart => "UART",
            Self::I2c => "I2C",
            Self::Spi => "SPI",
            Self::I2s => "I2S",
            Self::Ddsss => "DDSSS",
        }
    }

    pub fn all_selectable() -> &'static [BusKind] {
        &[
            Self::Off,
            Self::Uart,
            Self::I2c,
            Self::Spi,
            Self::I2s,
            Self::Ddsss,
        ]
    }
}

/// One decoded frame / transaction on the time axis.
#[derive(Debug, Clone, Default)]
pub struct BusFrame {
    pub t_start: f64,
    pub t_end: f64,
    /// Compact plot label (packet name). Keep short — hex lives on the byte row.
    pub summary: String,
    pub bytes: Vec<u8>,
    /// DDSSS ASK integrity. Other buses leave this as `None`.
    pub error: BusFrameError,
}

impl BusFrame {
    pub fn plot_label(&self) -> String {
        match self.error {
            BusFrameError::None => self.summary.clone(),
            BusFrameError::Checksum => format!("{}!", self.summary),
            BusFrameError::Parity => format!("{} P!", self.summary),
            BusFrameError::Framing => format!("{} F!", self.summary),
        }
    }

    pub fn has_error(&self) -> bool {
        self.error != BusFrameError::None
    }
}

/// ASK packet integrity (DDSSS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BusFrameError {
    #[default]
    None,
    Checksum,
    Parity,
    Framing,
}

/// Byte-lane integrity mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BusByteError {
    #[default]
    None,
    Checksum,
    Parity,
    Framing,
}

/// One recognized DDSSS chip on the time axis (`None` = uncertain).
/// `one` is polarity-corrected to match decoded data bits (Table 2 ONE/ZERO).
/// `error` is a mismatch vs the spreading sequence for the decided bit
/// (Table 4 still allows several chip errors per bit).
#[derive(Debug, Clone, Copy)]
pub struct BusChipMark {
    pub t_start: f64,
    pub t_end: f64,
    pub one: Option<bool>,
    pub error: bool,
}

/// One correlator data bit (11 bits per Qi serial byte: start, b0..b7, parity, stop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusBitKind {
    Start,
    Data { index: u8 },
    Parity,
    Stop,
}

#[derive(Debug, Clone, Copy)]
pub struct BusBitMark {
    pub t_start: f64,
    pub t_end: f64,
    pub one: bool,
    pub kind: BusBitKind,
    pub error: bool,
}

impl BusBitMark {
    pub fn label(&self) -> String {
        let base = match self.kind {
            BusBitKind::Start => "St".into(),
            BusBitKind::Data { index } => format!("b{index}"),
            BusBitKind::Parity => "P".into(),
            BusBitKind::Stop => "Sp".into(),
        };
        if self.error {
            format!("{base}!")
        } else {
            base
        }
    }
}

/// One assembled Qi byte and the waveform interval that produced it.
#[derive(Debug, Clone)]
pub struct BusByteSpan {
    pub t_start: f64,
    pub t_end: f64,
    pub byte: u8,
    pub error: BusByteError,
}

impl BusByteSpan {
    pub fn label(&self) -> String {
        if self.error == BusByteError::None {
            format!("{:02X}", self.byte)
        } else {
            format!("{:02X}!", self.byte)
        }
    }
}

/// Hard cap on decoded payload bytes (all bus protocols). Further data is skipped.
pub const MAX_DECODE_BYTES: usize = 512;

pub(crate) fn try_push_frame(
    frames: &mut Vec<BusFrame>,
    used: &mut usize,
    frame: BusFrame,
) -> bool {
    let n = frame.bytes.len();
    if n > 0 && *used >= MAX_DECODE_BYTES {
        return false;
    }
    if n > 0 && *used + n > MAX_DECODE_BYTES {
        return false;
    }
    *used = used.saturating_add(n);
    frames.push(frame);
    true
}

/// Result of a bus decode pass.
#[derive(Debug, Clone, Default)]
pub struct BusDecodeResult {
    pub frames: Vec<BusFrame>,
    pub info: String,
    pub error: Option<String>,
    pub truncated: bool,
    /// DDSSS chip decisions (empty for other buses).
    pub chips: Vec<BusChipMark>,
    /// DDSSS correlator bits (empty for other buses).
    pub bits: Vec<BusBitMark>,
    /// DDSSS / framed bytes with waveform intervals (empty for other buses).
    pub byte_spans: Vec<BusByteSpan>,
}

/// Channel indices into the loaded `waves` vector.
#[derive(Debug, Clone, Default)]
pub struct BusChannelMap {
    pub uart_signal: Option<usize>,
    pub i2c_scl: Option<usize>,
    pub i2c_sda: Option<usize>,
    pub spi_clk: Option<usize>,
    pub spi_mosi: Option<usize>,
    pub spi_miso: Option<usize>,
    pub spi_cs: Option<usize>,
    pub spi_io2: Option<usize>,
    pub spi_io3: Option<usize>,
    pub i2s_bclk: Option<usize>,
    pub i2s_ws: Option<usize>,
    pub i2s_data: Option<usize>,
    pub ddsss_signal: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct BusDecodeSettings {
    pub kind: BusKind,
    pub channels: BusChannelMap,
    pub uart: UartConfig,
    pub i2c: I2cConfig,
    pub spi: SpiConfig,
    pub i2s: I2sConfig,
    pub ddsss: DdsssConfig,
    pub threshold: Option<f64>,
    pub idle_high: bool,
    /// Optional time gate `[t0, t1]` (seconds, trace time base).
    pub time_gate: Option<(f64, f64)>,
}

impl Default for BusDecodeSettings {
    fn default() -> Self {
        Self {
            kind: BusKind::Off,
            channels: BusChannelMap::default(),
            uart: UartConfig::default(),
            i2c: I2cConfig::default(),
            spi: SpiConfig::default(),
            i2s: I2sConfig::default(),
            ddsss: DdsssConfig::default(),
            threshold: None,
            idle_high: true,
            time_gate: None,
        }
    }
}

/// Run decode on loaded traces. Always uses full-resolution samples.
pub fn decode_bus(waves: &[WaveformTrace], settings: &BusDecodeSettings) -> BusDecodeResult {
    if settings.kind == BusKind::Off {
        return BusDecodeResult::default();
    }

    let slice_trace = |trace: &WaveformTrace| -> WaveformTrace {
        gate_trace(trace, settings.time_gate)
    };

    match settings.kind {
        BusKind::Off => BusDecodeResult::default(),
        BusKind::Uart => {
            let Some(idx) = settings.channels.uart_signal else {
                return need_channel("UART signal");
            };
            let Some(trace) = waves.get(idx) else {
                return invalid_channel();
            };
            decode_uart(&slice_trace(trace), settings.threshold, settings.idle_high, &settings.uart)
        }
        BusKind::I2c => {
            let (Some(scl_i), Some(sda_i)) = (settings.channels.i2c_scl, settings.channels.i2c_sda)
            else {
                return need_channel("I2C SCL + SDA");
            };
            let (Some(scl), Some(sda)) = (waves.get(scl_i), waves.get(sda_i)) else {
                return invalid_channel();
            };
            decode_i2c(
                &slice_trace(scl),
                &slice_trace(sda),
                settings.threshold,
                &settings.i2c,
            )
        }
        BusKind::Spi => {
            let Some(clk_i) = settings.channels.spi_clk else {
                return need_channel("SPI CLK");
            };
            let Some(clk) = waves.get(clk_i) else {
                return invalid_channel();
            };
            let io0 = settings
                .channels
                .spi_mosi
                .and_then(|i| waves.get(i))
                .map(|t| slice_trace(t));
            let io1 = settings
                .channels
                .spi_miso
                .and_then(|i| waves.get(i))
                .map(|t| slice_trace(t));
            let io2 = settings
                .channels
                .spi_io2
                .and_then(|i| waves.get(i))
                .map(|t| slice_trace(t));
            let io3 = settings
                .channels
                .spi_io3
                .and_then(|i| waves.get(i))
                .map(|t| slice_trace(t));
            let cs = settings
                .channels
                .spi_cs
                .and_then(|i| waves.get(i))
                .map(|t| slice_trace(t));
            match settings.spi.wire {
                SpiWire::TwoWire | SpiWire::FourWire if io0.is_none() => {
                    return need_channel("SPI CLK + MOSI");
                }
                SpiWire::Dual if io0.is_none() || io1.is_none() => {
                    return need_channel("SPI CLK + IO0 + IO1");
                }
                SpiWire::Quad if io0.is_none() || io1.is_none() || io2.is_none() || io3.is_none() => {
                    return need_channel("SPI CLK + IO0 + IO1 + IO2 + IO3");
                }
                _ => {}
            }
            decode_spi(
                &slice_trace(clk),
                io0.as_ref(),
                io1.as_ref(),
                io2.as_ref(),
                io3.as_ref(),
                cs.as_ref(),
                settings.threshold,
                &settings.spi,
            )
        }
        BusKind::I2s => {
            let (Some(bclk_i), Some(ws_i), Some(data_i)) = (
                settings.channels.i2s_bclk,
                settings.channels.i2s_ws,
                settings.channels.i2s_data,
            ) else {
                return need_channel("I2S BCLK + WS + DATA");
            };
            let (Some(bclk), Some(ws), Some(data)) = (
                waves.get(bclk_i),
                waves.get(ws_i),
                waves.get(data_i),
            ) else {
                return invalid_channel();
            };
            decode_i2s(
                &slice_trace(bclk),
                &slice_trace(ws),
                &slice_trace(data),
                settings.threshold,
                &settings.i2s,
            )
        }
        BusKind::Ddsss => {
            let Some(idx) = settings.channels.ddsss_signal else {
                return need_channel("DDSSS signal");
            };
            let Some(trace) = waves.get(idx) else {
                return invalid_channel();
            };
            decode_ddsss(&slice_trace(trace), &settings.ddsss)
        }
    }
}

/// Remap channel indices and return only the source wave indices to clone.
pub fn compact_bus_decode_indices(
    wave_count: usize,
    settings: &BusDecodeSettings,
) -> (Vec<usize>, BusDecodeSettings) {
    let mut unique: Vec<usize> = Vec::new();
    let mut remap = |idx: Option<usize>| -> Option<usize> {
        let i = idx?;
        if i >= wave_count {
            return None;
        }
        if let Some(pos) = unique.iter().position(|&u| u == i) {
            Some(pos)
        } else {
            unique.push(i);
            Some(unique.len() - 1)
        }
    };

    let mut compact = settings.clone();
    let ch = &mut compact.channels;
    ch.uart_signal = remap(ch.uart_signal);
    ch.i2c_scl = remap(ch.i2c_scl);
    ch.i2c_sda = remap(ch.i2c_sda);
    ch.spi_clk = remap(ch.spi_clk);
    ch.spi_mosi = remap(ch.spi_mosi);
    ch.spi_miso = remap(ch.spi_miso);
    ch.spi_cs = remap(ch.spi_cs);
    ch.spi_io2 = remap(ch.spi_io2);
    ch.spi_io3 = remap(ch.spi_io3);
    ch.i2s_bclk = remap(ch.i2s_bclk);
    ch.i2s_ws = remap(ch.i2s_ws);
    ch.i2s_data = remap(ch.i2s_data);
    ch.ddsss_signal = remap(ch.ddsss_signal);

    (unique, compact)
}

/// Clone only the traces referenced by channel map (reduces memory for background decode).
pub fn compact_bus_decode_input(
    waves: &[WaveformTrace],
    settings: &BusDecodeSettings,
) -> (Vec<WaveformTrace>, BusDecodeSettings) {
    let (unique, compact) = compact_bus_decode_indices(waves.len(), settings);
    let traces: Vec<WaveformTrace> = unique.iter().filter_map(|&i| waves.get(i).cloned()).collect();
    (traces, compact)
}

fn gate_trace(trace: &WaveformTrace, gate: Option<(f64, f64)>) -> WaveformTrace {
    let Some((t0, t1)) = gate else {
        return trace.clone();
    };
    let lo = t0.min(t1);
    let hi = t0.max(t1);
    let n = trace.x.len().min(trace.y.len());
    if n == 0 {
        return trace.clone();
    }
    let start = trace.x[..n].partition_point(|&v| v < lo);
    let end = trace.x[..n]
        .partition_point(|&v| v <= hi)
        .max(start + 1)
        .min(n);
    WaveformTrace {
        channel: trace.channel.clone(),
        x: trace.x[start..end].to_vec().into(),
        y: trace.y[start..end].to_vec().into(),
        x_unit: trace.x_unit.clone(),
        y_unit: trace.y_unit.clone(),
    }
}

fn need_channel(what: &str) -> BusDecodeResult {
    BusDecodeResult {
        frames: Vec::new(),
        info: String::new(),
        error: Some(format!("Assign channel(s): {what}")),
        ..Default::default()
    }
}

fn invalid_channel() -> BusDecodeResult {
    BusDecodeResult {
        frames: Vec::new(),
        info: String::new(),
        error: Some("Selected channel index out of range".into()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_trace_limits_samples() {
        let trace = WaveformTrace {
            channel: "CH1".into(),
            x: (0..100).map(|i| i as f64 * 1e-6).collect::<Vec<_>>().into(),
            y: vec![0.0; 100].into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let gated = gate_trace(&trace, Some((20e-6, 50e-6)));
        assert!(gated.x.len() < trace.x.len());
        assert!(gated.x.first().unwrap() >= &20e-6);
    }

    #[test]
    fn decode_bus_ddsss_ce() {
        // Tiny carrier: reuse the public decoder via BusKind.
        let fop = 128_000.0;
        let spc = 8usize;
        let dt = 1.0 / (fop * spc as f64);
        // Not a valid packet — just ensure the kind dispatches without panic.
        let n = 256;
        let x: Vec<f64> = (0..n).map(|i| i as f64 * dt).collect();
        let y: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * (i % spc) as f64 / spc as f64).sin())
            .collect();
        let trace = WaveformTrace {
            channel: "CH1".into(),
            x: x.into(),
            y: y.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let mut settings = BusDecodeSettings::default();
        settings.kind = BusKind::Ddsss;
        settings.channels.ddsss_signal = Some(0);
        settings.ddsss.fop_hz = Some(fop);
        let r = decode_bus(&[trace], &settings);
        assert!(r.error.is_none(), "{r:?}");
    }
}
