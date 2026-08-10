//! I2S decode (Philips, MSB-first, sample on BCLK rising edge).

use super::digital::{analog_to_edges, default_threshold, level_at_time, EdgeKind};
use super::{BusDecodeResult, BusFrame};
use crate::instrument::WaveformTrace;

#[derive(Debug, Clone)]
pub struct I2sConfig {
    pub bits_per_sample: u8,
}

impl Default for I2sConfig {
    fn default() -> Self {
        Self {
            bits_per_sample: 16,
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
        };
    }

    let bits = cfg.normalized_bits();
    let thr_bclk = threshold.unwrap_or_else(|| default_threshold(bclk));
    let thr_ws = threshold.unwrap_or_else(|| default_threshold(ws));
    let thr_data = threshold.unwrap_or_else(|| default_threshold(data));
    let bclk_edges = analog_to_edges(bclk, thr_bclk, 0.08);

    let mut frames = Vec::new();
    let mut word: u32 = 0;
    let mut bit_count = 0u8;
    let mut t_word_start = 0.0;
    let mut ws_at_word: Option<bool> = None;

    for e in &bclk_edges {
        if e.kind != EdgeKind::Rising {
            continue;
        }
        let t = e.time;
        if bit_count == 0 {
            t_word_start = t;
            ws_at_word = level_at_time(ws, thr_ws, t);
        }

        let data_bit = level_at_time(data, thr_data, t).unwrap_or(false);
        word = (word << 1) | u32::from(data_bit);
        bit_count += 1;

        if bit_count == bits {
            let ch = match ws_at_word {
                Some(true) => "R",
                Some(false) => "L",
                None => "?",
            };
            let hex_w = (bits as usize + 3) / 4;
            let summary = format!("I2S {ch} 0x{word:0hex_w$X}");
            let bytes = sample_to_bytes(word, bits);
            frames.push(BusFrame {
                t_start: t_word_start,
                t_end: t,
                summary,
                bytes,
            });
            word = 0;
            bit_count = 0;
            ws_at_word = None;
            if frames.len() >= 512 {
                break;
            }
        }
    }

    let info = format!(
        "I2S Philips {bits}-bit: {} sample(s)",
        frames.len()
    );
    let empty = frames.is_empty();
    BusDecodeResult {
        frames,
        info,
        error: if empty {
            Some("No I2S samples decoded — check BCLK/WS/DATA".into())
        } else {
            None
        },
    }
}

fn sample_to_bytes(word: u32, bits: u8) -> Vec<u8> {
    match bits {
        32 => word.to_be_bytes().to_vec(),
        24 => vec![((word >> 16) & 0xFF) as u8, ((word >> 8) & 0xFF) as u8, (word & 0xFF) as u8],
        _ => ((word & 0xFFFF) as u16).to_be_bytes().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_i2s_sample(
        value: u16,
        bits_per_word: usize,
        samples_per_bit: usize,
    ) -> (WaveformTrace, WaveformTrace, WaveformTrace) {
        let dt = 1e-7;
        let mut t = 0.0;
        let mut x = Vec::new();
        let mut bclk_y = Vec::new();
        let mut ws_y = Vec::new();
        let mut data_y = Vec::new();

        for bit in 0..bits_per_word {
            let mask = 1 << (bits_per_word - 1 - bit);
            let high = value as u32 & mask != 0;
            let data_v = if high { 3.3 } else { 0.0 };
            for phase in 0..samples_per_bit {
                x.push(t);
                ws_y.push(0.0);
                data_y.push(data_v);
                let bclk_v = if phase < samples_per_bit / 2 {
                    0.0
                } else {
                    3.3
                };
                bclk_y.push(bclk_v);
                t += dt;
            }
        }

        let mk = |channel: &str, y: Vec<f64>| WaveformTrace {
            channel: channel.into(),
            x: x.clone().into(),
            y,
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        (
            mk("BCLK", bclk_y),
            mk("WS", ws_y),
            mk("DATA", data_y),
        )
    }

    #[test]
    fn decodes_synthetic_i2s_left_sample() {
        let (bclk, ws, data) = synth_i2s_sample(0x55AA, 16, 4);
        let cfg = I2sConfig::default();
        let r = decode_i2s(&bclk, &ws, &data, None, &cfg);
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(!r.frames.is_empty());
        assert!(r.frames[0].summary.contains("L"));
        assert_eq!(r.frames[0].bytes, vec![0x55, 0xAA]);
    }
}
