//! I2C decode from SCL + SDA analog traces.

use super::digital::{analog_to_edges, default_threshold, level_at_time, EdgeKind};
use super::{BusDecodeResult, BusFrame};
use crate::instrument::WaveformTrace;

pub fn decode_i2c(
    scl: &WaveformTrace,
    sda: &WaveformTrace,
    threshold: Option<f64>,
    _idle_high: bool,
) -> BusDecodeResult {
    let n = scl.x.len().min(scl.y.len()).min(sda.x.len()).min(sda.y.len());
    if n < 32 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("Trace too short for I2C decode".into()),
        };
    }

    let thr_scl = threshold.unwrap_or_else(|| default_threshold(scl));
    let thr_sda = threshold.unwrap_or_else(|| default_threshold(sda));
    let scl_edges = analog_to_edges(scl, thr_scl, 0.08);

    let mut frames = Vec::new();
    let mut i = 0usize;
    while i + 1 < scl_edges.len() {
        let rise = &scl_edges[i];
        if rise.kind != EdgeKind::Rising {
            i += 1;
            continue;
        }
        let t = rise.time;
        let sda_before = level_at_time(sda, thr_sda, t - 1e-9).unwrap_or(true);
        let sda_at = level_at_time(sda, thr_sda, t).unwrap_or(true);

        // START: SDA falls while SCL high (sample just before rising edge)
        if sda_before && !sda_at {
            if let Some((frame, next_i)) = read_i2c_transaction(scl, sda, thr_scl, thr_sda, &scl_edges, i) {
                frames.push(frame);
                i = next_i;
                continue;
            }
        }
        i += 1;
    }

    let info = format!("I2C: {} transaction(s)", frames.len());
    let empty = frames.is_empty();
    BusDecodeResult {
        frames,
        info,
        error: if empty {
            Some("No I2C START detected — check SCL/SDA assignment".into())
        } else {
            None
        },
    }
}

fn read_i2c_transaction(
    scl: &WaveformTrace,
    sda: &WaveformTrace,
    thr_scl: f64,
    thr_sda: f64,
    scl_edges: &[super::digital::DigitalEdge],
    start_edge_i: usize,
) -> Option<(BusFrame, usize)> {
    let t_start = scl_edges[start_edge_i].time;
    let mut bits = Vec::new();
    let mut i = start_edge_i + 1;

    while i < scl_edges.len() {
        let e = &scl_edges[i];
        if e.kind != EdgeKind::Rising {
            i += 1;
            continue;
        }
        let t = e.time;
        let scl_high_before_stop = level_at_time(scl, thr_scl, t).unwrap_or(false);
        let sda_level = level_at_time(sda, thr_sda, t).unwrap_or(true);

        // STOP: SDA rises while SCL high
        if i + 1 < scl_edges.len() {
            let sda_prev = level_at_time(sda, thr_sda, t - 1e-9).unwrap_or(false);
            if scl_high_before_stop && !sda_prev && sda_level {
                break;
            }
        }

        bits.push(sda_level);
        i += 1;
        if bits.len() > 9 * 256 {
            break;
        }
    }

    if bits.len() < 9 {
        return None;
    }

    let mut bytes = Vec::new();
    let mut idx = 0;
    while idx + 8 <= bits.len() {
        let mut b = 0u8;
        for bit in 0..8 {
            if bits[idx + bit] {
                b |= 1 << (7 - bit);
            }
        }
        bytes.push(b);
        idx += 9; // 8 data + ACK slot
        if bytes.len() >= 32 {
            break;
        }
    }

    if bytes.is_empty() {
        return None;
    }

    let addr = bytes[0] >> 1;
    let rw = if bytes[0] & 1 == 1 { "R" } else { "W" };
    let data_hex: String = bytes
        .iter()
        .skip(1)
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let summary = if data_hex.is_empty() {
        format!("Addr 0x{addr:02X} {rw}")
    } else {
        format!("Addr 0x{addr:02X} {rw}  [{data_hex}]")
    };

    let t_end = scl_edges.get(i).map(|e| e.time).unwrap_or(t_start);
    Some((
        BusFrame {
            t_start,
            t_end,
            summary,
            bytes,
        },
        i,
    ))
}
