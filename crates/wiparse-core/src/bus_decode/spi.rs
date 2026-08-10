//! SPI decode (Mode 0, MSB-first, 8-bit words).

use super::digital::{analog_to_edges, default_threshold, level_at_time, EdgeKind};
use super::{BusDecodeResult, BusFrame};
use crate::instrument::WaveformTrace;

pub fn decode_spi(
    clk: &WaveformTrace,
    mosi: &WaveformTrace,
    miso: Option<&WaveformTrace>,
    cs: Option<&WaveformTrace>,
    threshold: Option<f64>,
    idle_high: bool,
) -> BusDecodeResult {
    let n = clk.x.len().min(clk.y.len()).min(mosi.x.len()).min(mosi.y.len());
    if n < 16 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("Trace too short for SPI decode".into()),
        };
    }

    let thr_clk = threshold.unwrap_or_else(|| default_threshold(clk));
    let thr_mosi = threshold.unwrap_or_else(|| default_threshold(mosi));
    let thr_cs = cs.map(|t| threshold.unwrap_or_else(|| default_threshold(t)));
    let thr_miso = miso.map(|t| threshold.unwrap_or_else(|| default_threshold(t)));
    let has_miso = miso.is_some();

    let clk_edges = analog_to_edges(clk, thr_clk, 0.08);
    let mut frames = Vec::new();
    let mut mosi_word: u8 = 0;
    let mut miso_word: u8 = 0;
    let mut bit_count = 0u8;
    let mut t_word_start = 0.0;
    let mut in_frame = cs.is_none();

    for e in &clk_edges {
        if e.kind != EdgeKind::Rising {
            continue;
        }
        let t = e.time;

        if let (Some(cs_trace), Some(thr)) = (cs, thr_cs) {
            let cs_active = level_at_time(cs_trace, thr, t).unwrap_or(idle_high);
            let active = if idle_high { !cs_active } else { cs_active };
            if !active {
                if in_frame && bit_count > 0 {
                    push_spi_word(&mut frames, t_word_start, t, mosi_word, miso_word, has_miso);
                }
                in_frame = false;
                mosi_word = 0;
                miso_word = 0;
                bit_count = 0;
                continue;
            }
            if !in_frame {
                in_frame = true;
                t_word_start = t;
                mosi_word = 0;
                miso_word = 0;
                bit_count = 0;
            }
        }

        if !in_frame {
            continue;
        }

        let mosi_bit = level_at_time(mosi, thr_mosi, t).unwrap_or(false);
        mosi_word = (mosi_word << 1) | u8::from(mosi_bit);
        if let (Some(miso_trace), Some(tm)) = (miso, thr_miso) {
            let miso_bit = level_at_time(miso_trace, tm, t).unwrap_or(false);
            miso_word = (miso_word << 1) | u8::from(miso_bit);
        }
        bit_count += 1;

        if bit_count == 8 {
            let mut summary = format!("MOSI 0x{mosi_word:02X}");
            if has_miso {
                summary.push_str(&format!(" / MISO 0x{miso_word:02X}"));
            }
            let mut bytes = vec![mosi_word];
            if has_miso {
                bytes.push(miso_word);
            }
            frames.push(BusFrame {
                t_start: t_word_start,
                t_end: t,
                summary,
                bytes,
            });
            mosi_word = 0;
            miso_word = 0;
            bit_count = 0;
            t_word_start = t;
        }
    }

    let info = format!("SPI Mode 0: {} word(s)", frames.len());
    let empty = frames.is_empty();
    BusDecodeResult {
        frames,
        info,
        error: if empty {
            Some("No SPI words decoded — check CLK/MOSI/CS".into())
        } else {
            None
        },
    }
}

fn push_spi_word(
    frames: &mut Vec<BusFrame>,
    t0: f64,
    t1: f64,
    mosi: u8,
    miso: u8,
    has_miso: bool,
) {
    let summary = if has_miso {
        format!("MOSI 0x{mosi:02X} / MISO 0x{miso:02X} (partial)")
    } else {
        format!("MOSI 0x{mosi:02X} (partial)")
    };
    let mut bytes = vec![mosi];
    if has_miso {
        bytes.push(miso);
    }
    frames.push(BusFrame {
        t_start: t0,
        t_end: t1,
        summary,
        bytes,
    });
}
