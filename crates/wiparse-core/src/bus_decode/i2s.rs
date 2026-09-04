//! I2S decode.
//!
//! Philips (I2S standard): WS changes one BCLK before MSB; WS=0 left, WS=1 right.
//! Left-justified: MSB aligned with the WS edge (no 1-bit delay).
//! Sample SD on BCLK rising (data launched on falling, per NXP I2S spec).

use super::digital::{EdgeKind, LogicWave};
use super::{try_push_frame, BusDecodeResult, BusFrame, MAX_DECODE_BYTES};
use crate::instrument::WaveformTrace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum I2sFormat {
    #[default]
    Philips,
    LeftJustified,
}

impl I2sFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Philips => "Philips",
            Self::LeftJustified => "Left-J",
        }
    }

    fn msb_delay_clocks(self) -> u8 {
        match self {
            Self::Philips => 1,
            Self::LeftJustified => 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct I2sConfig {
    pub bits_per_sample: u8,
    pub format: I2sFormat,
}

impl Default for I2sConfig {
    fn default() -> Self {
        Self {
            bits_per_sample: 16,
            format: I2sFormat::Philips,
        }
    }
}

impl I2sConfig {
    pub fn normalized_bits(&self) -> u8 {
        match self.bits_per_sample {
            24 => 24,
            32 => 32,
            _ => 16,
        }
    }
}

enum Evt {
    Ws { t: f64, high: bool },
    Bclk(f64),
}

pub fn decode_i2s(
    bclk: &WaveformTrace,
    ws: &WaveformTrace,
    data: &WaveformTrace,
    threshold: Option<f64>,
    cfg: &I2sConfig,
) -> BusDecodeResult {
    let n = bclk
        .x
        .len()
        .min(bclk.y.len())
        .min(ws.x.len())
        .min(ws.y.len())
        .min(data.x.len())
        .min(data.y.len());
    if n < 32 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("Trace too short for I2S decode".into()),
            ..Default::default()
        };
    }

    let bits = cfg.normalized_bits();
    let bclk_w = LogicWave::new(bclk, threshold);
    let ws_w = LogicWave::new(ws, threshold);
    let data_w = LogicWave::new(data, threshold);
    let delay = cfg.format.msb_delay_clocks();

    let mut events = Vec::new();
    for e in bclk_w.edges() {
        if e.kind == EdgeKind::Rising {
            events.push(Evt::Bclk(e.time));
        }
    }
    for e in ws_w.edges() {
        events.push(Evt::Ws {
            t: e.time,
            high: e.kind == EdgeKind::Rising,
        });
    }
    events.sort_by(|a, b| {
        evt_t(a)
            .partial_cmp(&evt_t(b))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| evt_pri(a).cmp(&evt_pri(b)))
    });

    let mut frames = Vec::new();
    let mut used = 0usize;
    let mut truncated = false;
    let mut word: u32 = 0;
    let mut bit_count = 0u8;
    let mut t_word_start = 0.0;
    let mut skip = 0u8;
    let mut collecting = false;
    let mut ws_high = false;
    let mut pending_new = false;
    let mut pending_ws_high = false;
    let mut have_ws = false;

    for ev in &events {
        if truncated {
            break;
        }
        match *ev {
            Evt::Ws { t, high } => {
                if delay == 0 && collecting && bit_count > 0 && bit_count < bits {
                    truncated |= !push_sample(
                        &mut frames,
                        &mut used,
                        word,
                        bit_count,
                        bits,
                        t_word_start,
                        t,
                        ws_high,
                        true,
                    );
                    word = 0;
                    bit_count = 0;
                    collecting = false;
                }
                have_ws = true;
                pending_ws_high = high;
                pending_new = true;
                skip = delay;
                if delay == 0 {
                    collecting = true;
                    ws_high = high;
                    pending_new = false;
                    word = 0;
                    bit_count = 0;
                }
            }
            Evt::Bclk(t) => {
                if !have_ws {
                    continue;
                }
                // Finish the previous word first. In Philips, the first clock after
                // WS is the old LSB (the 1-bit delay) when the slot is only N clocks.
                if collecting && bit_count > 0 && bit_count < bits {
                    let data_bit = data_w.before(t).unwrap_or(false);
                    word = (word << 1) | u32::from(data_bit);
                    bit_count += 1;
                    if pending_new && skip > 0 {
                        skip -= 1;
                    }
                    if bit_count >= bits {
                        truncated |= !push_sample(
                            &mut frames,
                            &mut used,
                            word,
                            bit_count,
                            bits,
                            t_word_start,
                            t,
                            ws_high,
                            false,
                        );
                        word = 0;
                        bit_count = 0;
                        collecting = false;
                    }
                    continue;
                }
                if pending_new {
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    collecting = true;
                    ws_high = pending_ws_high;
                    pending_new = false;
                    word = 0;
                    bit_count = 0;
                }
                if !collecting {
                    continue;
                }
                if bit_count >= bits {
                    continue;
                }
                if bit_count == 0 {
                    t_word_start = t;
                }
                let data_bit = data_w.before(t).unwrap_or(false);
                word = (word << 1) | u32::from(data_bit);
                bit_count += 1;
                if bit_count >= bits {
                    truncated |= !push_sample(
                        &mut frames,
                        &mut used,
                        word,
                        bit_count,
                        bits,
                        t_word_start,
                        t,
                        ws_high,
                        false,
                    );
                    word = 0;
                    bit_count = 0;
                    collecting = false;
                }
            }
        }
    }

    let mut info = format!(
        "I2S {} {bits}-bit: {} sample(s)",
        cfg.format.label(),
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
            Some("No I2S samples decoded — check BCLK/WS/DATA and wait for a WS edge".into())
        } else {
            None
        },
        truncated,
        ..Default::default()
    }
}

fn push_sample(
    frames: &mut Vec<BusFrame>,
    used: &mut usize,
    word: u32,
    bit_count: u8,
    bits: u8,
    t0: f64,
    t1: f64,
    ws_high: bool,
    partial: bool,
) -> bool {
    if bit_count == 0 {
        return true;
    }
    let ch = if ws_high { "R" } else { "L" };
    let hex_w = (bits as usize + 3) / 4;
    let summary = if partial || bit_count < bits {
        format!("I2S {ch} 0x{word:X} (partial {bit_count}b)")
    } else {
        format!("I2S {ch} 0x{word:0hex_w$X}")
    };
    try_push_frame(
        frames,
        used,
        BusFrame {
            t_start: t0,
            t_end: t1,
            summary,
            bytes: sample_to_bytes(word, bits),
            ..Default::default()
        },
    )
}

fn evt_t(e: &Evt) -> f64 {
    match *e {
        Evt::Ws { t, .. } | Evt::Bclk(t) => t,
    }
}

fn evt_pri(e: &Evt) -> u8 {
    // Same timestamp: WS change first so Philips delay consumes the next (or this) BCLK.
    match e {
        Evt::Ws { .. } => 0,
        Evt::Bclk(_) => 1,
    }
}

fn sample_to_bytes(word: u32, bits: u8) -> Vec<u8> {
    match bits {
        32 => word.to_be_bytes().to_vec(),
        24 => vec![
            ((word >> 16) & 0xFF) as u8,
            ((word >> 8) & 0xFF) as u8,
            (word & 0xFF) as u8,
        ],
        _ => ((word & 0xFFFF) as u16).to_be_bytes().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold(
        x: &mut Vec<f64>,
        bclk: &mut Vec<f64>,
        ws: &mut Vec<f64>,
        data: &mut Vec<f64>,
        t: &mut f64,
        dt: f64,
        n: usize,
        bclk_v: f64,
        ws_v: f64,
        data_v: f64,
    ) {
        for _ in 0..n {
            x.push(*t);
            bclk.push(bclk_v);
            ws.push(ws_v);
            data.push(data_v);
            *t += dt;
        }
    }

    fn clock_bit(
        x: &mut Vec<f64>,
        bclk: &mut Vec<f64>,
        ws: &mut Vec<f64>,
        data: &mut Vec<f64>,
        t: &mut f64,
        dt: f64,
        n: usize,
        ws_v: f64,
        bit: bool,
    ) {
        let data_v = if bit { 3.3 } else { 0.0 };
        hold(x, bclk, ws, data, t, dt, n, 0.0, ws_v, data_v);
        hold(x, bclk, ws, data, t, dt, n, 3.3, ws_v, data_v);
        hold(x, bclk, ws, data, t, dt, n, 0.0, ws_v, data_v);
    }

    /// Philips: WS falls, one dummy BCLK, then 16 data bits (left, WS=0).
    fn synth_philips_left(value: u16) -> (WaveformTrace, WaveformTrace, WaveformTrace) {
        let dt = 1e-7;
        let n = 4usize;
        let mut t = 0.0;
        let mut x = Vec::new();
        let mut bclk = Vec::new();
        let mut ws = Vec::new();
        let mut data = Vec::new();

        hold(&mut x, &mut bclk, &mut ws, &mut data, &mut t, dt, n * 2, 0.0, 3.3, 0.0);
        // WS falling while BCLK low (standard).
        hold(&mut x, &mut bclk, &mut ws, &mut data, &mut t, dt, n, 0.0, 3.3, 0.0);
        hold(&mut x, &mut bclk, &mut ws, &mut data, &mut t, dt, n, 0.0, 0.0, 0.0);
        // Dummy BCLK (1-bit delay) — not MSB.
        clock_bit(&mut x, &mut bclk, &mut ws, &mut data, &mut t, dt, n, 0.0, false);
        for i in (0..16).rev() {
            clock_bit(
                &mut x,
                &mut bclk,
                &mut ws,
                &mut data,
                &mut t,
                dt,
                n,
                0.0,
                (value >> i) & 1 == 1,
            );
        }
        hold(&mut x, &mut bclk, &mut ws, &mut data, &mut t, dt, n * 2, 0.0, 0.0, 0.0);

        mk(x, bclk, ws, data)
    }

    fn mk(
        x: Vec<f64>,
        bclk: Vec<f64>,
        ws: Vec<f64>,
        data: Vec<f64>,
    ) -> (WaveformTrace, WaveformTrace, WaveformTrace) {
        let trace = |ch: &str, y: Vec<f64>| WaveformTrace {
            channel: ch.into(),
            x: x.clone().into(),
            y: y.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        (trace("BCLK", bclk), trace("WS", ws), trace("DATA", data))
    }

    #[test]
    fn philips_skips_first_clock_and_labels_left() {
        let (bclk, ws, data) = synth_philips_left(0x55AA);
        let r = decode_i2s(&bclk, &ws, &data, None, &I2sConfig::default());
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(!r.frames.is_empty(), "{:?}", r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>());
        assert!(r.frames[0].summary.contains("L"), "{}", r.frames[0].summary);
        assert_eq!(r.frames[0].bytes, vec![0x55, 0xAA]);
    }

    #[test]
    fn left_justified_does_not_skip_msb() {
        let (bclk, ws, data) = synth_philips_left(0x55AA);
        let cfg = I2sConfig {
            bits_per_sample: 16,
            format: I2sFormat::LeftJustified,
        };
        let r = decode_i2s(&bclk, &ws, &data, None, &cfg);
        // Dummy 0 + 15 MSBs of 0x55AA → not 0x55AA.
        assert!(
            r.frames
                .iter()
                .all(|f| f.bytes != vec![0x55, 0xAA]),
            "Left-J must not match Philips alignment: {:?}",
            r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>()
        );
    }
}
