//! I2C decode from SCL + SDA analog traces.
//!
//! START: SDA falls while SCL is high.
//! STOP:  SDA rises while SCL is high.
//! Data:  SDA sampled on SCL rising edge (MSB first, 8 bits + ACK).

use super::digital::{EdgeKind, LogicWave};
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
    if n < 16 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("Trace too short for I2C decode".into()),
            ..Default::default()
        };
    }

    let scl_w = LogicWave::new(scl, threshold);
    let sda_w = LogicWave::new(sda, threshold);
    let result = decode_i2c_oriented(&scl_w, &sda_w, cfg);
    if payload_bytes(&result) > 0 {
        return result;
    }
    // Wrong SCL/SDA assignment often still yields START/STOP (clock edges on SDA)
    // but no address/data. Retry the swapped orientation and keep it only if it
    // actually decodes payload.
    let mut swapped = decode_i2c_oriented(&sda_w, &scl_w, cfg);
    if payload_bytes(&swapped) > payload_bytes(&result) {
        if !swapped.info.is_empty() {
            swapped.info = format!("I2C (SCL/SDA swapped): {}", swapped.info);
        }
        return swapped;
    }
    result
}

fn payload_bytes(result: &BusDecodeResult) -> usize {
    result.frames.iter().map(|f| f.bytes.len()).sum()
}

fn decode_i2c_oriented(
    scl: &LogicWave<'_>,
    sda: &LogicWave<'_>,
    cfg: &I2cConfig,
) -> BusDecodeResult {
    let events = collect_events(scl, sda);
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
        if !matches!(events[i].kind, EvtKind::SdaFall) || !scl_stable_high(scl, events[i].t)
        {
            i += 1;
            continue;
        }
        match read_transaction(
            scl,
            sda,
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

fn collect_events(scl: &LogicWave<'_>, sda: &LogicWave<'_>) -> Vec<Evt> {
    let scl_edges = scl.edges();
    let mut events = Vec::with_capacity(scl_edges.len() + 16);
    for e in scl_edges {
        if e.kind == EdgeKind::Rising {
            events.push(Evt {
                t: e.time,
                kind: EvtKind::SclRise,
            });
        }
    }
    for e in sda.edges() {
        events.push(Evt {
            t: e.time,
            kind: if e.kind == EdgeKind::Rising {
                EvtKind::SdaRise
            } else {
                EvtKind::SdaFall
            },
        });
    }
    if let Some(t0) = sda.trace.x.first().copied() {
        let scl_hi = scl.first_volts().is_some_and(|v| v >= scl.levels.vih);
        let sda_lo = sda.first_volts().is_some_and(|v| v <= sda.levels.vil);
        // Capture often triggers on the first clock, so START is already complete
        // (SCL high, SDA low) at t0 with no falling edge in the window.
        if scl_hi && sda_lo {
            let dt = sample_dt(sda.trace);
            let has_fall = events
                .iter()
                .any(|e| matches!(e.kind, EvtKind::SdaFall) && (e.t - t0).abs() <= dt);
            if !has_fall {
                events.push(Evt {
                    t: t0,
                    kind: EvtKind::SdaFall,
                });
            }
        }
    }
    events.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    events
}

fn scl_stable_high(scl: &LogicWave<'_>, t: f64) -> bool {
    match (scl.at(t), scl.before(t)) {
        (Some(now), Some(before)) => now && before,
        (Some(now), None) => now,
        _ => false,
    }
}

fn sample_dt(trace: &WaveformTrace) -> f64 {
    let n = trace.x.len();
    if n >= 2 {
        ((trace.x[n - 1] - trace.x[0]) / (n as f64 - 1.0)).abs().max(1e-15)
    } else {
        1e-9
    }
}

fn ack_label(ack: bool) -> &'static str {
    if ack {
        "ACK"
    } else {
        "NAK"
    }
}

fn read_transaction(
    scl: &LogicWave<'_>,
    sda: &LogicWave<'_>,
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
    let mut expect_10bit_low = false;

    while i < events.len() {
        let ev = &events[i];
        match ev.kind {
            EvtKind::SdaFall if scl_stable_high(scl, ev.t) => {
                // Repeated START: leave this event for the outer loop.
                return Some((i, truncated));
            }
            EvtKind::SdaRise if scl_stable_high(scl, ev.t) => {
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
                let sample = sda.before(ev.t).unwrap_or(true);
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
                        &mut expect_10bit_low,
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
                                EvtKind::SdaFall if scl_stable_high(scl, ev.t) => {
                                    flush_partial_reg(
                                        frames,
                                        used,
                                        cfg,
                                        &mut reg_needed,
                                        reg_value,
                                        reg_t0,
                                        ev.t,
                                    );
                                    return Some((i, truncated));
                                }
                                EvtKind::SdaRise if scl_stable_high(scl, ev.t) => {
                                    flush_partial_reg(
                                        frames,
                                        used,
                                        cfg,
                                        &mut reg_needed,
                                        reg_value,
                                        reg_t0,
                                        ev.t,
                                    );
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
    expect_10bit_low: &mut bool,
    reg_needed: &mut usize,
    reg_value: &mut u16,
    reg_t0: &mut f64,
) -> bool {
    let ack_s = ack_label(ack);
    if byte_index == 0 {
        // 10-bit address prefix: 11110xxR (0xF0..0xF7).
        if value & 0xF8 == 0xF0 {
            *is_write = value & 1 == 0;
            // Second address byte follows a write prefix only. After Sr the
            // master repeats 11110xx1 and then data — not the low address byte.
            *expect_10bit_low = *is_write;
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
        *expect_10bit_low = false;
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

    if *expect_10bit_low {
        *expect_10bit_low = false;
        let hi = frames
            .iter()
            .rev()
            .find(|f| f.summary.starts_with("10-bit"))
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

    fn synth_ops(ops: &[I2cOp]) -> (WaveformTrace, WaveformTrace) {
        let dt = 1e-7;
        let n = 4usize;
        let mut t = 0.0;
        let mut x = Vec::new();
        let mut scl = Vec::new();
        let mut sda = Vec::new();
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n * 2, 3.3, 3.3);
        for op in ops {
            match *op {
                I2cOp::Start => {
                    hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 3.3, 3.3);
                    hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 3.3, 0.0);
                    hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 0.0, 0.0);
                }
                I2cOp::Stop => {
                    hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 0.0, 0.0);
                    hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 3.3, 0.0);
                    hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 3.3, 3.3);
                }
                I2cOp::Byte(b, ack) => {
                    clock_byte(&mut x, &mut scl, &mut sda, &mut t, dt, b, ack, n);
                }
            }
        }
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

    #[derive(Clone, Copy)]
    enum I2cOp {
        Start,
        Stop,
        Byte(u8, bool),
    }

    fn synth_bytes(bytes: &[u8]) -> (WaveformTrace, WaveformTrace) {
        let mut ops = vec![I2cOp::Start];
        for &b in bytes {
            ops.push(I2cOp::Byte(b, true));
        }
        ops.push(I2cOp::Stop);
        synth_ops(&ops)
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

    fn synth_after_start(addr: u8, payload: &[u8]) -> (WaveformTrace, WaveformTrace) {
        let dt = 1e-7;
        let n = 4usize;
        let mut t = 0.0;
        let mut x = Vec::new();
        let mut scl = Vec::new();
        let mut sda = Vec::new();
        // Window opens after START: SCL already high, SDA already low.
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n * 2, 3.3, 0.0);
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 0.0, 0.0);
        let mut bytes = vec![addr << 1];
        bytes.extend_from_slice(payload);
        for &b in &bytes {
            clock_byte(&mut x, &mut scl, &mut sda, &mut t, dt, b, true, n);
        }
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 0.0, 0.0);
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 3.3, 0.0);
        hold(&mut x, &mut scl, &mut sda, &mut t, dt, n, 3.3, 3.3);
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
    fn start_inferred_when_capture_begins_after_start_bit() {
        let (scl, sda) = synth_after_start(0x50, &[0x12]);
        let r = decode_i2c(&scl, &sda, None, &I2cConfig { reg_addr_bits: 8 });
        assert!(r.error.is_none(), "{:?}", r.error);
        let texts: Vec<&str> = r.frames.iter().map(|f| f.summary.as_str()).collect();
        assert!(texts.contains(&"START"), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Dev 0x50 W")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Reg 0x12")), "{texts:?}");
    }

    #[test]
    fn swapped_scl_sda_still_decodes() {
        let (scl, sda) = synth_write(0x3C, &[0x00]);
        let r = decode_i2c(&sda, &scl, None, &I2cConfig::default());
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(
            r.frames.iter().any(|f| f.summary.contains("Dev 0x3C")),
            "{:?}",
            r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>()
        );
        assert!(r.info.contains("swapped"), "{}", r.info);
    }

    #[test]
    fn write_then_repeated_start_read() {
        let (scl, sda) = synth_ops(&[
            I2cOp::Start,
            I2cOp::Byte(0x50 << 1, true),
            I2cOp::Byte(0x12, true),
            I2cOp::Start,
            I2cOp::Byte((0x50 << 1) | 1, true),
            I2cOp::Byte(0xAB, false),
            I2cOp::Stop,
        ]);
        let r = decode_i2c(&scl, &sda, None, &I2cConfig { reg_addr_bits: 8 });
        assert!(r.error.is_none(), "{:?}", r.error);
        let texts: Vec<&str> = r.frames.iter().map(|f| f.summary.as_str()).collect();
        assert!(texts.contains(&"START"), "{texts:?}");
        assert!(texts.contains(&"Sr"), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Dev 0x50 W")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Reg 0x12")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Dev 0x50 R")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Data 0xAB NAK")), "{texts:?}");
        assert!(texts.contains(&"STOP"), "{texts:?}");
    }

    #[test]
    fn ten_bit_read_after_sr_does_not_eat_data_as_address() {
        let (scl, sda) = synth_ops(&[
            I2cOp::Start,
            I2cOp::Byte(0xF4, true),
            I2cOp::Byte(0xA3, true),
            I2cOp::Start,
            I2cOp::Byte(0xF5, true),
            I2cOp::Byte(0x77, false),
            I2cOp::Stop,
        ]);
        let r = decode_i2c(&scl, &sda, None, &I2cConfig { reg_addr_bits: 8 });
        let texts: Vec<&str> = r.frames.iter().map(|f| f.summary.as_str()).collect();
        assert!(texts.iter().any(|s| s.contains("Dev 10b 0x2A3 W")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("10-bit R")), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Data 0x77 NAK")), "{texts:?}");
        assert_eq!(
            texts.iter().filter(|s| s.contains("Dev 10b")).count(),
            1,
            "{texts:?}"
        );
    }

    #[test]
    fn nak_then_stop() {
        let (scl, sda) = synth_ops(&[
            I2cOp::Start,
            I2cOp::Byte(0x50 << 1, false),
            I2cOp::Stop,
        ]);
        let r = decode_i2c(&scl, &sda, None, &I2cConfig { reg_addr_bits: 8 });
        assert!(r.error.is_none(), "{:?}", r.error);
        let texts: Vec<&str> = r.frames.iter().map(|f| f.summary.as_str()).collect();
        assert!(texts.contains(&"START"), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("Dev 0x50 W") && s.contains("NAK")), "{texts:?}");
        assert!(texts.contains(&"STOP"), "{texts:?}");
        assert!(!texts.iter().any(|s| s.contains("Data")), "{texts:?}");
    }

    #[test]
    fn midband_sda_ringing_is_not_start_stop() {
        let (scl0, sda0) = synth_write(0x50, &[0x12, 0xAB]);
        let dt = if scl0.x.len() >= 2 {
            scl0.x[1] - scl0.x[0]
        } else {
            1e-7
        };
        let n = 24usize;
        let mut x = Vec::with_capacity(n + scl0.x.len());
        let mut yscl = Vec::with_capacity(n + scl0.y.len());
        let mut ysda = Vec::with_capacity(n + sda0.y.len());
        let mut t = scl0.x.first().copied().unwrap_or(0.0) - dt * n as f64;
        let ring = [1.2, 1.9, 1.3, 2.0, 1.4, 1.8];
        for i in 0..n {
            x.push(t);
            yscl.push(3.3);
            ysda.push(ring[i % ring.len()]);
            t += dt;
        }
        x.extend(scl0.x.iter().copied());
        yscl.extend(scl0.y.iter().copied());
        ysda.extend(sda0.y.iter().copied());
        let scl = WaveformTrace {
            channel: "SCL".into(),
            x: x.clone().into(),
            y: yscl.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let sda = WaveformTrace {
            channel: "SDA".into(),
            x: x.into(),
            y: ysda.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let r = decode_i2c(&scl, &sda, None, &I2cConfig { reg_addr_bits: 8 });
        assert!(r.error.is_none(), "{:?}", r.error);
        let n_start = r
            .frames
            .iter()
            .filter(|f| f.summary == "START" || f.summary == "Sr")
            .count();
        assert_eq!(
            n_start,
            1,
            "{:?}",
            r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>()
        );
        assert!(r.frames.iter().any(|f| f.summary.contains("Dev 0x50 W")));
        assert!(r.frames.iter().any(|f| f.summary.contains("Data 0xAB")));
    }

    #[test]
    fn millivolt_analog_jitter_is_not_i2c() {
        let n = 200usize;
        let dt = 1e-7;
        let mut x = Vec::with_capacity(n);
        let mut scl = Vec::with_capacity(n);
        let mut sda = Vec::with_capacity(n);
        for i in 0..n {
            x.push(i as f64 * dt);
            scl.push(0.03 * (((i * 3) % 7) as f64 - 3.0) / 3.0);
            sda.push(0.02 * (((i * 5) % 9) as f64 - 4.0) / 4.0);
        }
        let scl = WaveformTrace {
            channel: "SCL".into(),
            x: x.clone().into(),
            y: scl.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let sda = WaveformTrace {
            channel: "SDA".into(),
            x: x.into(),
            y: sda.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let r = decode_i2c(&scl, &sda, None, &I2cConfig::default());
        assert!(
            r.frames.is_empty(),
            "jitter decoded as {:?}",
            r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>()
        );
    }
}
