//! I2C decode from SCL + SDA analog traces.
//!
//! START: SDA falls while SCL is high.
//! STOP:  SDA rises while SCL is high.
//! Data:  SDA sampled on SCL rising edge (MSB first, 8 bits + ACK).

use super::digital::{analog_to_edges, default_threshold, level_before, EdgeKind};
use super::{try_push_frame, BusDecodeResult, BusFrame, MAX_DECODE_BYTES};
use crate::instrument::WaveformTrace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct I2cConfig {
    /// Register / data-address width after a 7-bit write address: 8 or 16.
    pub reg_addr_bits: u8,
}

impl Default for I2cConfig {
    fn default() -> Self {
        Self { reg_addr_bits: 8 }
    }
}

impl I2cConfig {
    pub fn normalized_reg_bits(self) -> u8 {
        if self.reg_addr_bits == 16 {
            16
        } else {
            8
        }
    }

    fn reg_byte_count(self) -> usize {
        if self.normalized_reg_bits() == 16 {
            2
        } else {
            1
        }
    }
}

#[derive(Clone, Copy)]
enum EvtKind {
    SclRise,
    SdaFall,
    SdaRise,
}

struct Evt {
    t: f64,
    kind: EvtKind,
}

pub fn decode_i2c(
    scl: &WaveformTrace,
    sda: &WaveformTrace,
    threshold: Option<f64>,
    cfg: &I2cConfig,
) -> BusDecodeResult {
    let n = scl.x.len().min(scl.y.len()).min(sda.x.len()).min(sda.y.len());
    if n < 32 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("Trace too short for I2C decode".into()),
            ..Default::default()
        };
    }

    let thr_scl = threshold.unwrap_or_else(|| default_threshold(scl));
    let thr_sda = threshold.unwrap_or_else(|| default_threshold(sda));
    let events = collect_events(scl, sda, thr_scl, thr_sda);
    if events.is_empty() {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("No I2C edges detected — check SCL/SDA assignment".into()),
            ..Default::default()
        };
    }

    let mut frames = Vec::new();
    let mut used = 0usize;
    let mut truncated = false;
    let mut i = 0usize;
    while i < events.len() {
        if used >= MAX_DECODE_BYTES {
            truncated = true;
            break;
        }
        if !matches!(events[i].kind, EvtKind::SdaFall) || !scl_held_high(scl, thr_scl, events[i].t) {
            i += 1;
            continue;
        }
        match read_transaction(
            scl,
            sda,
            thr_scl,
            thr_sda,
            &events,
            i,
            cfg,
            &mut frames,
            &mut used,
        ) {
            Some((next_i, more_cut)) => {
                truncated |= more_cut;
                i = next_i;
            }
            None => {
                i += 1;
            }
        }
    }

    let n_start = frames.iter().filter(|f| f.summary == "START" || f.summary == "Sr").count();
    let mut info = format!(
        "I2C: {} START, {}-bit reg, {} item(s)",
        n_start,
        cfg.normalized_reg_bits(),
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
            Some("No I2C START detected — check SCL/SDA assignment and threshold".into())
        } else {
            None
        },
        truncated,
    }
}

fn collect_events(
    scl: &WaveformTrace,
    sda: &WaveformTrace,
    thr_scl: f64,
    thr_sda: f64,
) -> Vec<Evt> {
    let scl_edges = analog_to_edges(scl, thr_scl, 0.08);
    let sda_edges = analog_to_edges(sda, thr_sda, 0.08);
    let mut events = Vec::with_capacity(scl_edges.len() + sda_edges.len());
    for e in scl_edges {
        if e.kind == EdgeKind::Rising {
            events.push(Evt {
                t: e.time,
                kind: EvtKind::SclRise,
            });
        }
    }
    for e in sda_edges {
        events.push(Evt {
            t: e.time,
            kind: match e.kind {
                EdgeKind::Falling => EvtKind::SdaFall,
                EdgeKind::Rising => EvtKind::SdaRise,
            },
        });
    }
    events.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    events
}

fn scl_high(scl: &WaveformTrace, thr: f64, t: f64) -> bool {
    level_before(scl, thr, t).unwrap_or(false)
}

fn scl_held_high(scl: &WaveformTrace, thr: f64, t: f64) -> bool {
    let dt = sample_dt(scl);
    scl_high(scl, thr, t) && scl_high(scl, thr, (t - dt).max(scl.x.first().copied().unwrap_or(t)))
}

fn sample_dt(trace: &WaveformTrace) -> f64 {
    let n = trace.x.len();
    if n >= 2 {
        ((trace.x[n - 1] - trace.x[0]) / (n as f64 - 1.0)).abs().max(1e-15)
    } else {
        1e-9
    }
}

fn sda_level(sda: &WaveformTrace, thr: f64, t: f64) -> bool {
    level_before(sda, thr, t).unwrap_or(true)
}

fn ack_label(ack: bool) -> &'static str {
    if ack {
        "ACK"
    } else {
        "NAK"
    }
}

fn read_transaction(
    scl: &WaveformTrace,
    sda: &WaveformTrace,
    thr_scl: f64,
    thr_sda: f64,
    events: &[Evt],
    start_i: usize,
    cfg: &I2cConfig,
    frames: &mut Vec<BusFrame>,
    used: &mut usize,
) -> Option<(usize, bool)> {
    let t_start = events[start_i].t;
    let start_label = if frames.iter().any(|f| f.summary == "START" || f.summary == "Sr")
        && frames.last().is_some_and(|f| f.summary != "STOP")
    {
        "Sr"
    } else {
        "START"
    };
    if !try_push_frame(
        frames,
        used,
        BusFrame {
            t_start,
            t_end: t_start,
            summary: start_label.into(),
            bytes: Vec::new(),
        },
    ) {
        return Some((start_i + 1, true));
    }

    let mut bits: u8 = 0;
    let mut value: u8 = 0;
    let mut t_byte0 = t_start;
    let mut byte_index: usize = 0;
    let mut is_write = true;
    let mut reg_needed: usize = 0;
    let mut reg_value: u16 = 0;
    let mut reg_t0 = t_start;
    let mut truncated = false;
    let mut i = start_i + 1;

    while i < events.len() {
        let ev = &events[i];
        match ev.kind {
            EvtKind::SdaFall if scl_held_high(scl, thr_scl, ev.t) => {
                // Repeated START: leave this event for the outer loop.
                return Some((i, truncated));
            }
            EvtKind::SdaRise if scl_held_high(scl, thr_scl, ev.t) => {
                flush_partial_reg(frames, used, cfg, &mut reg_needed, reg_value, reg_t0, ev.t);
                let _ = try_push_frame(
                    frames,
                    used,
                    BusFrame {
                        t_start: ev.t,
                        t_end: ev.t,
                        summary: "STOP".into(),
                        bytes: Vec::new(),
                    },
                );
                return Some((i + 1, truncated));
            }
            EvtKind::SclRise => {
                if ev.t <= t_start {
                    i += 1;
                    continue;
                }
                if bits == 0 {
                    t_byte0 = ev.t;
                }
                let sample = sda_level(sda, thr_sda, ev.t);
                if bits < 8 {
                    value = (value << 1) | u8::from(sample);
                    bits += 1;
                } else {
                    // 9th SCL rise: ACK = SDA low.
                    let ack = !sample;
                    let t_end = ev.t;
                    let pushed = emit_i2c_byte(
                        frames,
                        used,
                        cfg,
                        byte_index,
                        value,
                        ack,
                        t_byte0,
                        t_end,
                        &mut is_write,
                        &mut reg_needed,
                        &mut reg_value,
                        &mut reg_t0,
                    );
                    if !pushed {
                        return Some((i + 1, true));
                    }
                    bits = 0;
                    value = 0;
                    byte_index += 1;
                    if !ack {
                        // NAK: wait for STOP / Sr, ignore extra clocks.
                        i += 1;
                        while i < events.len() {
                            let ev = &events[i];
                            match ev.kind {
                                EvtKind::SdaFall if scl_held_high(scl, thr_scl, ev.t) => {
                                    return Some((i, truncated));
                                }
                                EvtKind::SdaRise if scl_held_high(scl, thr_scl, ev.t) => {
                                    let _ = try_push_frame(
                                        frames,
                                        used,
                                        BusFrame {
                                            t_start: ev.t,
                                            t_end: ev.t,
                                            summary: "STOP".into(),
                                            bytes: Vec::new(),
                                        },
                                    );
                                    return Some((i + 1, truncated));
                                }
                                _ => i += 1,
                            }
                        }
                        return Some((i, truncated));
                    }
                    if *used >= MAX_DECODE_BYTES {
                        truncated = true;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }

    flush_partial_reg(frames, used, cfg, &mut reg_needed, reg_value, reg_t0, t_byte0);
    Some((i, truncated))
}

fn emit_i2c_byte(
    frames: &mut Vec<BusFrame>,
    used: &mut usize,
    cfg: &I2cConfig,
    byte_index: usize,
    value: u8,
    ack: bool,
    t0: f64,
    t1: f64,
    is_write: &mut bool,
    reg_needed: &mut usize,
    reg_value: &mut u16,
    reg_t0: &mut f64,
) -> bool {
    let ack_s = ack_label(ack);
    if byte_index == 0 {
        // 10-bit address prefix: 11110xxR (0xF0..0xF7).
        if value & 0xF8 == 0xF0 {
            *is_write = value & 1 == 0;
            *reg_needed = 0;
            return try_push_frame(
                frames,
                used,
                BusFrame {
                    t_start: t0,
                    t_end: t1,
                    summary: format!(
                        "10-bit {} hi 0x{:02X} {ack_s}",
                        if *is_write { "W" } else { "R" },
                        (value >> 1) & 0x03
                    ),
                    bytes: vec![value],
                },
            );
        }
        let addr = value >> 1;
        *is_write = value & 1 == 0;
        *reg_needed = if *is_write { cfg.reg_byte_count() } else { 0 };
        *reg_value = 0;
        *reg_t0 = t0;
        let summary = if addr == 0 && *is_write {
            format!("General call {ack_s}")
        } else {
            let rw = if *is_write { "W" } else { "R" };
            format!("Dev 0x{addr:02X} {rw} {ack_s}")
        };
        return try_push_frame(
            frames,
            used,
            BusFrame {
                t_start: t0,
                t_end: t1,
                summary,
                bytes: vec![value],
            },
        );
    }

    if byte_index == 1 && frames.last().is_some_and(|f| f.summary.starts_with("10-bit")) {
        let hi = frames
            .last()
            .and_then(|f| f.bytes.first().copied())
            .map(|b| (b >> 1) & 0x03)
            .unwrap_or(0);
        let addr10 = (u16::from(hi) << 8) | u16::from(value);
        *reg_needed = if *is_write { cfg.reg_byte_count() } else { 0 };
        *reg_value = 0;
        *reg_t0 = t0;
        let rw = if *is_write { "W" } else { "R" };
        return try_push_frame(
            frames,
            used,
            BusFrame {
                t_start: t0,
                t_end: t1,
                summary: format!("Dev 10b 0x{addr10:03X} {rw} {ack_s}"),
                bytes: vec![value],
            },
        );
    }

    if *reg_needed > 0 {
        if *reg_needed == cfg.reg_byte_count() {
            *reg_t0 = t0;
        }
        *reg_value = (*reg_value << 8) | u16::from(value);
        *reg_needed -= 1;
        if *reg_needed == 0 {
            let summary = if cfg.normalized_reg_bits() == 16 {
                format!("Reg 0x{reg_value:04X} {ack_s}")
            } else {
                format!("Reg 0x{reg_value:02X} {ack_s}")
            };
            let bytes = if cfg.normalized_reg_bits() == 16 {
                vec![(*reg_value >> 8) as u8, (*reg_value & 0xFF) as u8]
            } else {
                vec![*reg_value as u8]
            };
            return try_push_frame(
                frames,
                used,
                BusFrame {
                    t_start: *reg_t0,
                    t_end: t1,
                    summary,
                    bytes,
                },
            );
        }
        return true;
    }

    try_push_frame(
        frames,
        used,
        BusFrame {
            t_start: t0,
            t_end: t1,
            summary: format!("Data 0x{value:02X} {ack_s}"),
            bytes: vec![value],
        },
    )
}

fn flush_partial_reg(
    frames: &mut Vec<BusFrame>,
    used: &mut usize,
    cfg: &I2cConfig,
    reg_needed: &mut usize,
    reg_value: u16,
    t0: f64,
    t1: f64,
) {
    if *reg_needed == 0 || *reg_needed == cfg.reg_byte_count() {
        return;
    }
    // 16-bit mode captured only the high byte.
    let _ = try_push_frame(
        frames,
        used,
        BusFrame {
            t_start: t0,
            t_end: t1,
            summary: format!("Reg 0x{reg_value:02X} (incomplete)"),
            bytes: vec![reg_value as u8],
        },
    );
    *reg_needed = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold(
        x: &mut Vec<f64>,
        scl: &mut Vec<f64>,
        sda: &mut Vec<f64>,
        t: &mut f64,
        dt: f64,
        n: usize,
        scl_v: f64,
        sda_v: f64,
    ) {
        for _ in 0..n {
            x.push(*t);
            scl.push(scl_v);
            sda.push(sda_v);
            *t += dt;
        }
    }

    fn clock_bit(
        x: &mut Vec<f64>,
        scl: &mut Vec<f64>,
        sda: &mut Vec<f64>,
        t: &mut f64,
        dt: f64,
        bit: bool,
        n: usize,
    ) {
        let sda_v = if bit { 3.3 } else { 0.0 };
        hold(x, scl, sda, t, dt, n, 0.0, sda_v);
        hold(x, scl, sda, t, dt, n, 3.3, sda_v);
        hold(x, scl, sda, t, dt, n, 0.0, sda_v);
    }

    fn clock_byte(
        x: &mut Vec<f64>,
        scl: &mut Vec<f64>,
        sda: &mut Vec<f64>,
        t: &mut f64,
        dt: f64,
        byte: u8,
        ack: bool,
        n: usize,
    ) {
        for i in (0..8).rev() {
            clock_bit(x, scl, sda, t, dt, (byte >> i) & 1 == 1, n);
        }
        clock_bit(x, scl, sda, t, dt, !ack, n);
    }

    fn synth_write(addr: u8, payload: &[u8]) -> (WaveformTrace, WaveformTrace) {
        let mut bytes = vec![addr << 1];
        bytes.extend_from_slice(payload);
        synth_bytes(&bytes)
    }

    fn synth_bytes(bytes: &[u8]) -> (WaveformTrace, WaveformTrace) {
        let dt = 1e-7;
        let n = 4usize;
        let mut t = 0.0;
        let mut x = Vec::new();
        let mut scl = Vec::new();
        let mut sda = Vec::new();

        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n * 2, 3.3, 3.3);
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 3.3, 3.3);
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 3.3, 0.0);
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 0.0, 0.0);

        for &b in bytes {
            clock_byte(&mut x, &mut scl, &mut sda, &mut t, dt, b, true, n);
        }

        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 0.0, 0.0);
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 3.3, 0.0);
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 3.3, 3.3);
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n * 2, 3.3, 3.3);

        let mk = |ch: &str, y: Vec<f64>| WaveformTrace {
            channel: ch.into(),
            x: x.clone().into(),
            y: y.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        (mk("SCL", scl), mk("SDA", sda))
    }

    #[test]
    fn decodes_start_addr_reg8_data_stop() {
        let (scl, sda) = synth_write(0x50, &[0x12, 0xAB]);
        let r = decode_i2c(&scl, &sda, None, &I2cConfig { reg_addr_bits: 8 });
        assert!(r.error.is_none(), "{:?}", r.error);
        let texts: Vec<&str> = r.frames.iter().map(|f| f.summary.as_str()).collect();
        assert!(texts.contains(&"START"), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Dev 0x50 W")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Reg 0x12")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Data 0xAB")), "{texts:?}");
        assert!(texts.contains(&"STOP"), "{texts:?}");
    }

    #[test]
    fn decodes_16bit_register_address() {
        let (scl, sda) = synth_write(0x50, &[0x12, 0x34, 0xAB]);
        let r = decode_i2c(&scl, &sda, None, &I2cConfig { reg_addr_bits: 16 });
        assert!(r.error.is_none(), "{:?}", r.error);
        let texts: Vec<&str> = r.frames.iter().map(|f| f.summary.as_str()).collect();
        assert!(texts.iter().any(|s| s.contains("Reg 0x1234")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Data 0xAB")), "{texts:?}");
        assert!(!texts.iter().any(|s| *s == "Reg 0x12 ACK"), "{texts:?}");
    }

    #[test]
    fn start_is_sda_fall_not_scl_rise() {
        let (scl, sda) = synth_write(0x3C, &[0x00]);
        let r = decode_i2c(&scl, &sda, None, &I2cConfig::default());
        assert!(r.frames.iter().any(|f| f.summary == "START"));
        let start = r.frames.iter().find(|f| f.summary == "START").unwrap();
        let dev = r.frames.iter().find(|f| f.summary.contains("Dev 0x3C")).unwrap();
        assert!(start.t_start < dev.t_start);
    }

    #[test]
    fn stops_at_512_payload_bytes() {
        let payload: Vec<u8> = (0..600).map(|i| i as u8).collect();
        let (scl, sda) = synth_write(0x50, &payload);
        let r = decode_i2c(&scl, &sda, None, &I2cConfig { reg_addr_bits: 8 });
        let nbytes: usize = r.frames.iter().map(|f| f.bytes.len()).sum();
        assert!(r.truncated);
        assert!(nbytes <= MAX_DECODE_BYTES, "decoded {nbytes} bytes");
        assert!(nbytes >= MAX_DECODE_BYTES.saturating_sub(2));
    }

    #[test]
    fn general_call_and_ten_bit_address() {
        let (scl, sda) = synth_bytes(&[0x00, 0x06]);
        let r = decode_i2c(&scl, &sda, None, &I2cConfig { reg_addr_bits: 8 });
        assert!(
            r.frames.iter().any(|f| f.summary.contains("General call")),
            "{:?}",
            r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>()
        );

        // 10-bit write address 0x2A3: prefix 11110_10_0 = 0xF4, then 0xA3.
        let (scl, sda) = synth_bytes(&[0xF4, 0xA3, 0x11]);
        let r = decode_i2c(&scl, &sda, None, &I2cConfig { reg_addr_bits: 8 });
        let texts: Vec<&str> = r.frames.iter().map(|f| f.summary.as_str()).collect();
        assert!(texts.iter().any(|s| s.contains("10-bit W")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Dev 10b 0x2A3")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Reg 0x11")), "{texts:?}");
    }
}
