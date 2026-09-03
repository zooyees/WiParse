//! UART (async NRZ) decode from a single analog trace.

use super::digital::{edge_intervals, EdgeKind, LogicWave};
use super::{try_push_frame, BusDecodeResult, BusFrame, MAX_DECODE_BYTES};
use crate::instrument::WaveformTrace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UartParity {
    #[default]
    None,
    Even,
    Odd,
    Mark,
    Space,
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

impl UartConfig {
    pub fn normalized_data_bits(&self) -> u8 {
        self.data_bits.clamp(5, 9)
    }

    pub fn normalized_stop_bits(&self) -> u8 {
        if self.stop_bits >= 2 {
            2
        } else {
            1
        }
    }
}

const STANDARD_BAUD: &[f64] = &[
    300.0, 600.0, 1_200.0, 2_400.0, 4_800.0, 9_600.0, 19_200.0, 38_400.0, 57_600.0, 115_200.0,
    230_400.0, 250_000.0, 460_800.0, 500_000.0, 921_600.0, 1_000_000.0, 1_500_000.0, 2_000_000.0,
    3_000_000.0,
];

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
            ..Default::default()
        };
    }

    let wave = LogicWave::new(trace, threshold);
    let edges = wave.edges();
    // A 0x00 / 0xFF frame has only two edges (start + stop). Require a start edge.
    if edges.len() < 2 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("No digital edges detected — check threshold / signal".into()),
            ..Default::default()
        };
    }

    let bit_time = match cfg.baud.filter(|&b| b > 0.0) {
        Some(b) => 1.0 / b,
        None => estimate_bit_time(trace, &edges).unwrap_or_else(|| {
            let tspan = (trace.x[n - 1] - trace.x[0]).abs().max(1e-12);
            tspan / (n as f64 * 0.1)
        }),
    };

    if !bit_time.is_finite() || bit_time <= 0.0 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("Could not estimate UART bit time".into()),
            ..Default::default()
        };
    }

    let idle = idle_high ^ cfg.inverted;
    let db = cfg.normalized_data_bits();
    let sb = cfg.normalized_stop_bits();
    let bits_total = 1.0 + f64::from(db) + parity_bits(cfg.parity) + f64::from(sb);

    let mut frames = Vec::new();
    let mut used = 0usize;
    let mut truncated = false;
    let mut next_t = f64::NEG_INFINITY;
    for e in edges.iter() {
        let is_start = if idle {
            e.kind == EdgeKind::Falling
        } else {
            e.kind == EdgeKind::Rising
        };
        if !is_start || e.time < next_t {
            continue;
        }
        if used >= MAX_DECODE_BYTES {
            truncated = true;
            break;
        }
        match decode_frame_at(&wave, e.time, bit_time, idle, cfg, bits_total) {
            FrameKind::Skip => {}
            FrameKind::Break(frame) => {
                // Stay in the break until the line returns to idle, otherwise a
                // long low pulse emits a chain of BREAK markers and hides data.
                next_t = next_idle_time(&edges, e.time, idle)
                    .unwrap_or(e.time + bits_total * bit_time);
                if !try_push_frame(&mut frames, &mut used, frame) {
                    truncated = true;
                    break;
                }
            }
            FrameKind::Data(frame) => {
                // Allow the next start at the end of the first stop bit, even if
                // the user selected 2 stop bits but the link uses 1.
                next_t = e.time + (1.0 + f64::from(db) + parity_bits(cfg.parity) + 0.80) * bit_time;
                if !try_push_frame(&mut frames, &mut used, frame) {
                    truncated = true;
                    break;
                }
            }
        }
    }

    let baud_est = 1.0 / bit_time;
    let auto = if cfg.baud.filter(|&b| b > 0.0).is_some() {
        ""
    } else {
        " (auto)"
    };
    let mut info = format!(
        "UART {}-{}-{} @ {:.0} baud{auto}, {} frame(s)",
        db,
        parity_label(cfg.parity),
        sb,
        baud_est,
        frames.len()
    );
    if truncated {
        info.push_str(&format!("; truncated at {MAX_DECODE_BYTES} bytes"));
    }

    let empty = frames.is_empty();
    BusDecodeResult {
        frames,
        info,
        error: if empty {
            Some("No UART frames decoded — check signal / baud / threshold".into())
        } else {
            None
        },
        truncated,
    }
}

fn parity_label(p: UartParity) -> &'static str {
    match p {
        UartParity::None => "N",
        UartParity::Even => "E",
        UartParity::Odd => "O",
        UartParity::Mark => "M",
        UartParity::Space => "S",
    }
}

fn parity_bits(p: UartParity) -> f64 {
    if p == UartParity::None {
        0.0
    } else {
        1.0
    }
}

fn next_idle_time(
    edges: &[super::digital::DigitalEdge],
    t_start: f64,
    idle_high: bool,
) -> Option<f64> {
    let idle_edge = if idle_high {
        EdgeKind::Rising
    } else {
        EdgeKind::Falling
    };
    edges
        .iter()
        .find(|e| e.time > t_start && e.kind == idle_edge)
        .map(|e| e.time)
}

fn estimate_bit_time(
    trace: &WaveformTrace,
    edges: &[super::digital::DigitalEdge],
) -> Option<f64> {
    let mut widths = edge_intervals(edges);
    let n = trace.x.len().min(trace.y.len());
    let sample_dt = if n >= 2 {
        (trace.x[n - 1] - trace.x[0]).abs() / (n as f64 - 1.0)
    } else {
        0.0
    };
    // Ignore 1-sample ringing; those would inflate baud and turn every start into BREAK.
    let min_width = (sample_dt * 1.8).max(1e-12);
    widths.retain(|w| w.is_finite() && *w >= min_width);
    if widths.is_empty() {
        return None;
    }
    widths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Drop the shortest ~5% as ringing / glitch pulses.
    let start = (widths.len() / 20).min(widths.len() - 1);
    let min_bit = widths[start];
    let ones: Vec<f64> = widths
        .iter()
        .copied()
        .filter(|w| *w <= min_bit * 1.45)
        .collect();
    let mut bt = ones[ones.len() / 2];
    if let Some(std) = snap_baud(1.0 / bt) {
        bt = 1.0 / std;
    }
    Some(bt)
}

fn snap_baud(est: f64) -> Option<f64> {
    if !est.is_finite() || est <= 0.0 {
        return None;
    }
    STANDARD_BAUD
        .iter()
        .copied()
        .find(|&nom| ((est - nom) / nom).abs() < 0.08)
}

enum FrameKind {
    Skip,
    Data(BusFrame),
    Break(BusFrame),
}

fn logic_one(level: bool, idle_high: bool) -> bool {
    if idle_high {
        level
    } else {
        !level
    }
}

fn decode_frame_at(
    wave: &LogicWave<'_>,
    t_start: f64,
    bit_time: f64,
    idle_high: bool,
    cfg: &UartConfig,
    bits_total: f64,
) -> FrameKind {
    let start_mid = t_start + 0.5 * bit_time;
    let Some(lvl) = wave.at(start_mid) else {
        return FrameKind::Skip;
    };
    if logic_one(lvl, idle_high) {
        return FrameKind::Skip;
    }

    let db = cfg.normalized_data_bits();
    let t_end = t_start + bits_total * bit_time;

    let mut all_zero = true;
    let mut data: u16 = 0;
    for bit in 0..db {
        let t = t_start + (1.5 + f64::from(bit)) * bit_time;
        let Some(lvl) = wave.at(t) else {
            return FrameKind::Skip;
        };
        let one = logic_one(lvl, idle_high);
        if one {
            all_zero = false;
            data |= 1 << bit;
        }
    }

    let mut parity_ok = true;
    let mut parity_bit = false;
    if cfg.parity != UartParity::None {
        let t = t_start + (1.5 + f64::from(db)) * bit_time;
        if let Some(lvl) = wave.at(t) {
            parity_bit = logic_one(lvl, idle_high);
            if parity_bit {
                all_zero = false;
            }
            let ones = (0..db).map(|b| (data >> b) & 1).sum::<u16>();
            let data_odd = ones % 2 == 1;
            parity_ok = match cfg.parity {
                // Even: total number of 1s (data + parity) is even → parity = data_odd.
                UartParity::Even => parity_bit == data_odd,
                // Odd: total number of 1s is odd → parity = !data_odd.
                UartParity::Odd => parity_bit != data_odd,
                UartParity::Mark => parity_bit,
                UartParity::Space => !parity_bit,
                UartParity::None => true,
            };
        }
    }

    let stop_offset = 1.5 + f64::from(db) + parity_bits(cfg.parity);
    let t_stop = t_start + stop_offset * bit_time;
    let stop_ok = wave
        .at(t_stop)
        .map(|lvl| logic_one(lvl, idle_high))
        .unwrap_or(false);

    // A real BREAK holds the line low through the stop bit and at least one extra
    // bit-time. `all_zero && !stop_ok` alone matches 0x00 when baud is slightly high.
    if all_zero && !stop_ok {
        let t_confirm = t_stop + bit_time;
        let still_low = wave
            .at(t_confirm)
            .map(|lvl| !logic_one(lvl, idle_high))
            .unwrap_or(true);
        if still_low {
            return FrameKind::Break(BusFrame {
                t_start,
                t_end: t_end.max(t_confirm),
                summary: "BREAK".into(),
                bytes: Vec::new(),
            });
        }
    }

    let ascii = if db <= 8 && data >= 0x20 && data < 0x7f {
        format!(" '{}'", data as u8 as char)
    } else {
        String::new()
    };
    let hex_w = if db > 8 { 3 } else { 2 };
    let mut summary = format!("0x{data:0hex_w$X}{ascii}");
    if !stop_ok {
        summary.push_str(" [framing]");
    }
    if !parity_ok {
        summary.push_str(" [parity]");
    }

    let bytes = if db > 8 {
        vec![(data & 0xFF) as u8, (data >> 8) as u8]
    } else {
        vec![(data & 0xFF) as u8]
    };
    let _ = parity_bit;
    FrameKind::Data(BusFrame {
        t_start,
        t_end,
        summary,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_uart_byte(byte: u8, baud: f64, samples_per_bit: usize) -> WaveformTrace {
        synth_uart_bytes(&[byte], baud, samples_per_bit, UartParity::None)
    }

    fn synth_uart_bytes(
        bytes: &[u8],
        baud: f64,
        samples_per_bit: usize,
        parity: UartParity,
    ) -> WaveformTrace {
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
        let hold = |x: &mut Vec<f64>, y: &mut Vec<f64>, t: &mut f64, level: f64| {
            for _ in 0..samples_per_bit {
                push(x, y, t, dt, level);
            }
        };
        hold(&mut x, &mut y, &mut t, 3.3);
        for &byte in bytes {
            hold(&mut x, &mut y, &mut t, 0.0);
            let mut ones = 0u8;
            for bit in 0..8 {
                let high = (byte >> bit) & 1 == 1;
                if high {
                    ones += 1;
                }
                hold(&mut x, &mut y, &mut t, if high { 3.3 } else { 0.0 });
            }
            match parity {
                UartParity::None => {}
                UartParity::Even => hold(&mut x, &mut y, &mut t, if ones % 2 == 1 { 3.3 } else { 0.0 }),
                UartParity::Odd => hold(&mut x, &mut y, &mut t, if ones % 2 == 0 { 3.3 } else { 0.0 }),
                UartParity::Mark => hold(&mut x, &mut y, &mut t, 3.3),
                UartParity::Space => hold(&mut x, &mut y, &mut t, 0.0),
            }
            hold(&mut x, &mut y, &mut t, 3.3);
        }
        hold(&mut x, &mut y, &mut t, 3.3);
        WaveformTrace {
            channel: "CH1".into(),
            x: x.into(),
            y: y.into(),
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

    #[test]
    fn skips_data_edges_inside_frame() {
        let trace = synth_uart_bytes(&[0xAA], 115_200.0, 8, UartParity::None);
        let r = decode_uart(&trace, None, true, &UartConfig::default());
        let data: Vec<_> = r.frames.iter().filter(|f| !f.bytes.is_empty()).collect();
        assert_eq!(data.len(), 1, "{:?}", r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>());
        assert_eq!(data[0].bytes[0], 0xAA);
    }

    #[test]
    fn even_parity_ok() {
        let trace = synth_uart_bytes(&[0x55], 115_200.0, 8, UartParity::Even);
        let mut cfg = UartConfig::default();
        cfg.parity = UartParity::Even;
        let r = decode_uart(&trace, None, true, &cfg);
        assert!(r.frames.iter().any(|f| f.bytes.first() == Some(&0x55) && !f.summary.contains("parity")));
    }

    #[test]
    fn even_parity_flags_wrong_bit() {
        // 0x55 has 4 ones (even) → even parity bit must be 0. Force a 1.
        let mut cfg = UartConfig::default();
        cfg.parity = UartParity::Even;
        let trace = synth_uart_bytes(&[0x55], 115_200.0, 8, UartParity::Odd);
        let r = decode_uart(&trace, None, true, &cfg);
        assert!(
            r.frames.iter().any(|f| f.summary.contains("parity")),
            "{:?}",
            r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>()
        );
    }

    #[test]
    fn decodes_all_zero_and_all_one_bytes() {
        let mut cfg = UartConfig::default();
        cfg.baud = Some(115_200.0);
        for byte in [0x00u8, 0xFF] {
            let trace = synth_uart_byte(byte, 115_200.0, 8);
            let r = decode_uart(&trace, None, true, &cfg);
            assert!(r.error.is_none(), "0x{byte:02X} {:?}", r.error);
            assert_eq!(
                r.frames[0].bytes[0],
                byte,
                "{:?}",
                r.frames[0].summary
            );
            assert!(!r.frames[0].summary.contains("framing"));
        }
    }

    #[test]
    fn two_stop_setting_does_not_skip_8n1_next_byte() {
        let trace = synth_uart_bytes(&[0x11, 0x22], 115_200.0, 8, UartParity::None);
        let mut cfg = UartConfig::default();
        cfg.stop_bits = 2;
        let r = decode_uart(&trace, None, true, &cfg);
        let data: Vec<u8> = r
            .frames
            .iter()
            .filter(|f| !f.bytes.is_empty())
            .map(|f| f.bytes[0])
            .collect();
        assert_eq!(data, vec![0x11, 0x22], "{:?}", r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>());
    }

    fn synth_uart_break_then_bytes(
        break_bits: usize,
        bytes: &[u8],
        baud: f64,
        samples_per_bit: usize,
    ) -> WaveformTrace {
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
        let hold = |x: &mut Vec<f64>, y: &mut Vec<f64>, t: &mut f64, bits: usize, level: f64| {
            for _ in 0..(bits * samples_per_bit) {
                push(x, y, t, dt, level);
            }
        };
        hold(&mut x, &mut y, &mut t, 2, 3.3);
        hold(&mut x, &mut y, &mut t, break_bits, 0.0);
        hold(&mut x, &mut y, &mut t, 3, 3.3);
        for &byte in bytes {
            hold(&mut x, &mut y, &mut t, 1, 0.0);
            for bit in 0..8 {
                let high = (byte >> bit) & 1 == 1;
                hold(&mut x, &mut y, &mut t, 1, if high { 3.3 } else { 0.0 });
            }
            hold(&mut x, &mut y, &mut t, 1, 3.3);
        }
        hold(&mut x, &mut y, &mut t, 2, 3.3);
        WaveformTrace {
            channel: "CH1".into(),
            x: x.into(),
            y: y.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        }
    }

    #[test]
    fn nul_byte_is_not_a_break() {
        let mut cfg = UartConfig::default();
        cfg.baud = Some(115_200.0);
        let trace = synth_uart_byte(0x00, 115_200.0, 8);
        let r = decode_uart(&trace, None, true, &cfg);
        assert_eq!(
            r.frames.iter().map(|f| f.summary.as_str()).collect::<Vec<_>>(),
            vec!["0x00"],
            "{:?}",
            r.frames
        );
    }

    #[test]
    fn break_does_not_hide_following_data() {
        let mut cfg = UartConfig::default();
        cfg.baud = Some(115_200.0);
        let trace = synth_uart_break_then_bytes(13, &[0x55, 0xAA], 115_200.0, 8);
        let r = decode_uart(&trace, None, true, &cfg);
        let texts: Vec<&str> = r.frames.iter().map(|f| f.summary.as_str()).collect();
        assert!(texts.iter().any(|s| *s == "BREAK"), "{texts:?}");
        let data: Vec<u8> = r
            .frames
            .iter()
            .filter(|f| !f.bytes.is_empty())
            .map(|f| f.bytes[0])
            .collect();
        assert_eq!(data, vec![0x55, 0xAA], "{texts:?}");
    }

    #[test]
    fn midband_ringing_does_not_create_start_bits() {
        let clean = synth_uart_byte(0x55, 115_200.0, 8);
        let dt = if clean.x.len() >= 2 {
            clean.x[1] - clean.x[0]
        } else {
            1e-6
        };
        let n = 32usize;
        let mut x = Vec::with_capacity(n + clean.x.len());
        let mut y = Vec::with_capacity(n + clean.y.len());
        let mut t = clean.x.first().copied().unwrap_or(0.0) - dt * n as f64;
        let ring = [1.2, 1.9, 1.3, 2.0, 1.4, 1.8];
        for i in 0..n {
            x.push(t);
            y.push(ring[i % ring.len()]);
            t += dt;
        }
        x.extend(clean.x.iter().copied());
        y.extend(clean.y.iter().copied());
        let trace = WaveformTrace {
            channel: "CH1".into(),
            x: x.into(),
            y: y.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let mut cfg = UartConfig::default();
        cfg.baud = Some(115_200.0);
        let r = decode_uart(&trace, None, true, &cfg);
        let data: Vec<u8> = r
            .frames
            .iter()
            .filter(|f| !f.bytes.is_empty())
            .map(|f| f.bytes[0])
            .collect();
        assert_eq!(data, vec![0x55], "{:?}", r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>());
    }
}
