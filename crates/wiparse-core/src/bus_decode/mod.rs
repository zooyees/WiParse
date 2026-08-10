//! Software bus decode from analog scope traces (offline waveform analysis).

mod digital;
mod i2c;
mod i2s;
mod spi;
mod uart;

pub use digital::{analog_to_edges, default_threshold, DigitalEdge, EdgeKind};
pub use i2c::decode_i2c;
pub use i2s::{decode_i2s, I2sConfig};
pub use spi::decode_spi;
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
}

impl BusKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Uart => "UART",
            Self::I2c => "I2C",
            Self::Spi => "SPI",
            Self::I2s => "I2S",
        }
    }

    pub fn all_selectable() -> &'static [BusKind] {
        &[Self::Off, Self::Uart, Self::I2c, Self::Spi, Self::I2s]
    }
}

/// One decoded frame / transaction on the time axis.
#[derive(Debug, Clone)]
pub struct BusFrame {
    pub t_start: f64,
    pub t_end: f64,
    pub summary: String,
    pub bytes: Vec<u8>,
}

/// Result of a bus decode pass.
#[derive(Debug, Clone, Default)]
pub struct BusDecodeResult {
    pub frames: Vec<BusFrame>,
    pub info: String,
    pub error: Option<String>,
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
    pub i2s_bclk: Option<usize>,
    pub i2s_ws: Option<usize>,
    pub i2s_data: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct BusDecodeSettings {
    pub kind: BusKind,
    pub channels: BusChannelMap,
    pub uart: UartConfig,
    pub i2s: I2sConfig,
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
            i2s: I2sConfig::default(),
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
                settings.idle_high,
            )
        }
        BusKind::Spi => {
            let (Some(clk_i), Some(mosi_i)) = (settings.channels.spi_clk, settings.channels.spi_mosi)
            else {
                return need_channel("SPI CLK + MOSI");
            };
            let clk = waves.get(clk_i);
            let mosi = waves.get(mosi_i);
            let (Some(clk), Some(mosi)) = (clk, mosi) else {
                return invalid_channel();
            };
            let miso = settings
                .channels
                .spi_miso
                .and_then(|i| waves.get(i))
                .map(|t| slice_trace(t));
            let cs = settings
                .channels
                .spi_cs
                .and_then(|i| waves.get(i))
                .map(|t| slice_trace(t));
            decode_spi(
                &slice_trace(clk),
                &slice_trace(mosi),
                miso.as_ref(),
                cs.as_ref(),
                settings.threshold,
                settings.idle_high,
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
    ch.i2s_bclk = remap(ch.i2s_bclk);
    ch.i2s_ws = remap(ch.i2s_ws);
    ch.i2s_data = remap(ch.i2s_data);

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
        y: trace.y[start..end].to_vec(),
        x_unit: trace.x_unit.clone(),
        y_unit: trace.y_unit.clone(),
    }
}

fn need_channel(what: &str) -> BusDecodeResult {
    BusDecodeResult {
        frames: Vec::new(),
        info: String::new(),
        error: Some(format!("Assign channel(s): {what}")),
    }
}

fn invalid_channel() -> BusDecodeResult {
    BusDecodeResult {
        frames: Vec::new(),
        info: String::new(),
        error: Some("Selected channel index out of range".into()),
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
            y: vec![0.0; 100],
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let gated = gate_trace(&trace, Some((20e-6, 50e-6)));
        assert!(gated.x.len() < trace.x.len());
        assert!(gated.x.first().unwrap() >= &20e-6);
    }
}
