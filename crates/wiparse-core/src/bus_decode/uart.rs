//! UART (async NRZ) decode from a single analog trace.

use super::digital::{analog_to_edges, default_threshold, edge_intervals, level_at_time, EdgeKind};
use super::{BusDecodeResult, BusFrame};
use crate::instrument::WaveformTrace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UartParity {
    None,
    Even,
    Odd,
}

#[derive(Debug, Clone)]
pub struct UartConfig {
    pub baud: Option<f64>,
    pub data_bits: u8,
    pub parity: UartParity,
    pub stop_bits: u8,
    pub inverted: bool,
}

impl Default for UartConfig {
    fn default() -> Self {
        Self {
            baud: None,
            data_bits: 8,
            parity: UartParity::None,
            stop_bits: 1,
            inverted: false,
        }
    }
}

pub fn decode_uart(
    trace: &WaveformTrace,
    threshold: Option<f64>,
    idle_high: bool,
    cfg: &UartConfig,
) -> BusDecodeResult {
    let n = trace.x.len().min(trace.y.len());
    if n < 16 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("Trace too short for UART decode".into()),
        };
    }

    let thr = threshold.unwrap_or_else(|| default_threshold(trace));
    let edges = analog_to_edges(trace, thr, 0.08);
    if edges.len() < 4 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("No digital edges detected — check threshold / signal".into()),
        };
    }

    let bit_time = match cfg.baud.filter(|&b| b > 0.0) {
        Some(b) => 1.0 / b,
        None => estimate_bit_time(&edges).unwrap_or_else(|| {
            let tspan = (trace.x[n - 1] - trace.x[0]).abs().max(1e-12);
            tspan / (n as f64 * 0.1)
        }),
    };

    if !bit_time.is_finite() || bit_time <= 0.0 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("Could not estimate UART bit time".into()),
        };
    }

    let mut frames = Vec::new();
    for (i, e) in edges.iter().enumerate() {
        let is_start = if idle_high ^ cfg.inverted {
            e.kind == EdgeKind::Falling
        } else {
            e.kind == EdgeKind::Rising
        };
        if !is_start {
            continue;
        }
        if let Some(frame) = decode_frame_at(trace, thr, e.time, bit_time, idle_high, cfg) {
            frames.push(frame);
        }
        let _ = i;
    }

    let baud_est = 1.0 / bit_time;
    let info = format!(
        "UART {}-{}-{} @ {:.0} baud (est.), {} frame(s)",
        cfg.data_bits,
        parity_label(cfg.parity),
        cfg.stop_bits,
        baud_est,
        frames.len()
    );

    let empty = frames.is_empty();
    BusDecodeResult {
        frames,
        info,
        error: if empty {
            Some("No UART frames decoded — check signal / baud / threshold".into())
        } else {
            None
        },
    }
}

fn parity_label(p: UartParity) -> &'static str {
    match p {
        UartParity::None => "N",
        UartParity::Even => "E",
        UartParity::Odd => "O",
    }
}

fn estimate_bit_time(edges: &[super::digital::DigitalEdge]) -> Option<f64> {
    let mut widths = edge_intervals(edges);
    if widths.is_empty() {
        return None;
    }
    widths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = widths.len() / 4;
    let mut bt = widths[idx];
    for &nom in &[1.0 / 115_200.0, 1.0 / 57_600.0, 1.0 / 38_400.0, 1.0 / 19_200.0, 1.0 / 9_600.0]
    {
        if (bt - nom).abs() < nom * 0.15 {
            bt = nom;
            break;
        }
    }
    Some(bt)
}

fn decode_frame_at(
    trace: &WaveformTrace,
    threshold: f64,
    t_start: f64,
    bit_time: f64,
    idle_high: bool,
    cfg: &UartConfig,
) -> Option<BusFrame> {
    let mut data: u8 = 0;
    let db = cfg.data_bits.min(8);
    for bit in 0..db {
        let t = t_start + (1.5 + f64::from(bit)) * bit_time;
        let lvl = level_at_time(trace, threshold, t)?;
        let one = if idle_high ^ cfg.inverted { lvl } else { !lvl };
        if one {
            data |= 1 << bit;
        }
    }

    let mut parity_ok = true;
    if cfg.parity != UartParity::None {
        let t = t_start + (1.5 + f64::from(db)) * bit_time;
        if let Some(lvl) = level_at_time(trace, threshold, t) {
            let bit = if idle_high ^ cfg.inverted { lvl } else { !lvl };
            let ones = (0..db).map(|b| (data >> b) & 1).sum::<u8>();
            parity_ok = match cfg.parity {
                UartParity::Even => bit == (ones % 2 == 0),
                UartParity::Odd => bit == (ones % 2 == 1),
                UartParity::None => true,
            };
        }
    }

    let stop_offset = 1.5 + f64::from(db) + if cfg.parity != UartParity::None { 1.0 } else { 0.0 };
    let t_stop = t_start + stop_offset * bit_time;
    let stop_ok = level_at_time(trace, threshold, t_stop)
        .map(|lvl| if idle_high ^ cfg.inverted { lvl } else { !lvl })
        .unwrap_or(false);

    let t_end = t_start + (stop_offset + f64::from(cfg.stop_bits)) * bit_time;
    let ascii = if data.is_ascii() && data >= 0x20 && data < 0x7f {
        format!(" '{}'", data as char)
    } else {
        String::new()
    };
    let mut summary = format!("0x{data:02X}{ascii}");
    if !stop_ok {
        summary.push_str(" [stop?]");
    }
    if !parity_ok {
        summary.push_str(" [parity?]");
    }

    Some(BusFrame {
        t_start,
        t_end,
        summary,
        bytes: vec![data],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_uart_byte(byte: u8, baud: f64, samples_per_bit: usize) -> WaveformTrace {
        let bit_t = 1.0 / baud;
        let mut x = Vec::new();
        let mut y = Vec::new();
        let mut t = 0.0;
        let dt = bit_t / samples_per_bit as f64;
        let push = |x: &mut Vec<f64>, y: &mut Vec<f64>, t: &mut f64, dt: f64, level: f64| {
            x.push(*t);
            y.push(level);
            *t += dt;
        };
        for _ in 0..samples_per_bit {
            push(&mut x, &mut y, &mut t, dt, 3.3);
        }
        for _ in 0..samples_per_bit {
            push(&mut x, &mut y, &mut t, dt, 0.0);
        }
        for bit in 0..8 {
            let high = (byte >> bit) & 1 == 1;
            let v = if high { 3.3 } else { 0.0 };
            for _ in 0..samples_per_bit {
                push(&mut x, &mut y, &mut t, dt, v);
            }
        }
        for _ in 0..samples_per_bit {
            push(&mut x, &mut y, &mut t, dt, 3.3);
        }
        WaveformTrace {
            channel: "CH1".into(),
            x: x.into(),
            y,
            x_unit: "s".into(),
            y_unit: "V".into(),
        }
    }

    #[test]
    fn decodes_synthetic_uart() {
        let trace = synth_uart_byte(0x55, 115_200.0, 8);
        let r = decode_uart(&trace, None, true, &UartConfig::default());
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(!r.frames.is_empty());
        assert_eq!(r.frames[0].bytes[0], 0x55);
    }
}
