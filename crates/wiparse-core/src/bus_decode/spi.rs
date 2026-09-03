//! SPI decode: 2-wire / 4-wire / Dual / Quad, modes 0–3.

use super::digital::{EdgeKind, LogicWave};
use super::{try_push_frame, BusDecodeResult, BusFrame, MAX_DECODE_BYTES};
use crate::instrument::WaveformTrace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpiWire {
    /// CLK + MOSI, optional CS (simplex).
    TwoWire,
    /// CLK + MOSI + MISO, optional CS (full duplex).
    #[default]
    FourWire,
    /// CLK + IO0 + IO1, optional CS (2 bits / clock).
    Dual,
    /// CLK + IO0..IO3, optional CS (4 bits / clock).
    Quad,
}

impl SpiWire {
    pub fn label(self) -> &'static str {
        match self {
            Self::TwoWire => "2-wire",
            Self::FourWire => "4-wire",
            Self::Dual => "Dual",
            Self::Quad => "Quad",
        }
    }

    pub fn bits_per_clock(self) -> u8 {
        match self {
            Self::TwoWire | Self::FourWire => 1,
            Self::Dual => 2,
            Self::Quad => 4,
        }
    }

    pub fn is_packed(self) -> bool {
        matches!(self, Self::Dual | Self::Quad)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpiMode {
    #[default]
    Mode0,
    Mode1,
    Mode2,
    Mode3,
}

impl SpiMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mode0 => "0",
            Self::Mode1 => "1",
            Self::Mode2 => "2",
            Self::Mode3 => "3",
        }
    }

    /// Sample on rising CLK (Mode 0 / 3) vs falling (Mode 1 / 2).
    pub fn sample_rising(self) -> bool {
        matches!(self, Self::Mode0 | Self::Mode3)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpiConfig {
    pub wire: SpiWire,
    pub mode: SpiMode,
    pub msb_first: bool,
    pub word_bits: u8,
    pub cs_active_low: bool,
}

impl Default for SpiConfig {
    fn default() -> Self {
        Self {
            wire: SpiWire::FourWire,
            mode: SpiMode::Mode0,
            msb_first: true,
            word_bits: 8,
            cs_active_low: true,
        }
    }
}

impl SpiConfig {
    pub fn normalized_word_bits(self) -> u8 {
        match self.word_bits {
            16 => 16,
            24 => 24,
            32 => 32,
            _ => 8,
        }
    }
}

struct Lane<'a> {
    wave: LogicWave<'a>,
}

enum Evt {
    CsOn(f64),
    CsOff(f64),
    Clk(f64),
}

pub fn decode_spi(
    clk: &WaveformTrace,
    io0: Option<&WaveformTrace>,
    io1: Option<&WaveformTrace>,
    io2: Option<&WaveformTrace>,
    io3: Option<&WaveformTrace>,
    cs: Option<&WaveformTrace>,
    threshold: Option<f64>,
    cfg: &SpiConfig,
) -> BusDecodeResult {
    let Some(io0) = io0 else {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("Assign SPI MOSI / IO0".into()),
            ..Default::default()
        };
    };
    if cfg.wire == SpiWire::Dual && io1.is_none() {
        return need("SPI Dual needs IO0 + IO1");
    }
    if cfg.wire == SpiWire::Quad && (io1.is_none() || io2.is_none() || io3.is_none()) {
        return need("SPI Quad needs IO0 + IO1 + IO2 + IO3");
    }
    let n = clk.x.len().min(clk.y.len()).min(io0.x.len()).min(io0.y.len());
    if n < 16 {
        return BusDecodeResult {
            frames: Vec::new(),
            info: String::new(),
            error: Some("Trace too short for SPI decode".into()),
            ..Default::default()
        };
    }

    let clk_w = LogicWave::new(clk, threshold);
    let lane0 = Lane {
        wave: LogicWave::new(io0, threshold),
    };
    let lane1 = io1.map(|t| Lane {
        wave: LogicWave::new(t, threshold),
    });
    let lane2 = io2.map(|t| Lane {
        wave: LogicWave::new(t, threshold),
    });
    let lane3 = io3.map(|t| Lane {
        wave: LogicWave::new(t, threshold),
    });
    let cs_w = cs.map(|t| LogicWave::new(t, threshold));

    let want_rise = cfg.mode.sample_rising();
    let clk_edges = clk_w.edges();
    let mut events = Vec::new();
    for e in &clk_edges {
        let is_sample = if want_rise {
            e.kind == EdgeKind::Rising
        } else {
            e.kind == EdgeKind::Falling
        };
        if is_sample {
            events.push(Evt::Clk(e.time));
        }
    }
    if let Some(cs_wave) = cs_w.as_ref() {
        for e in cs_wave.edges() {
            let on = if cfg.cs_active_low {
                e.kind == EdgeKind::Falling
            } else {
                e.kind == EdgeKind::Rising
            };
            events.push(if on { Evt::CsOn(e.time) } else { Evt::CsOff(e.time) });
        }
        // Same-time order: CS assert → CLK sample → CS deassert.
        events.sort_by(|a, b| {
            evt_t(a)
                .partial_cmp(&evt_t(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| evt_pri(a).cmp(&evt_pri(b)))
        });
    }

    let mut frames = Vec::new();
    let mut used = 0usize;
    let mut truncated = false;
    let mut cs_on = if let Some(cs_wave) = cs_w.as_ref() {
        let t0 = clk.x.first().copied().unwrap_or(0.0);
        let high = cs_wave.before(t0 + 1e-30).unwrap_or(!cfg.cs_active_low);
        if cfg.cs_active_low {
            !high
        } else {
            high
        }
    } else {
        true
    };
    let mut bit_count = 0u8;
    let mut mosi_word: u32 = 0;
    let mut miso_word: u32 = 0;
    let mut packed_word: u32 = 0;
    let mut t_word0 = 0.0;
    let word_bits = cfg.normalized_word_bits();
    let bpc = cfg.wire.bits_per_clock();
    let packed = cfg.wire.is_packed();
    let has_miso = lane1.is_some() && cfg.wire == SpiWire::FourWire;

    for ev in &events {
        if truncated {
            break;
        }
        match *ev {
            Evt::CsOn(t) => {
                flush_partial(
                    &mut frames,
                    &mut used,
                    cfg,
                    packed,
                    has_miso,
                    bit_count,
                    mosi_word,
                    miso_word,
                    packed_word,
                    t_word0,
                    t,
                );
                bit_count = 0;
                mosi_word = 0;
                miso_word = 0;
                packed_word = 0;
                cs_on = true;
                if !try_push_frame(
                    &mut frames,
                    &mut used,
                    BusFrame {
                        t_start: t,
                        t_end: t,
                        summary: "CS".into(),
                        bytes: Vec::new(),
                    },
                ) {
                    truncated = true;
                }
            }
            Evt::CsOff(t) => {
                truncated |= !flush_partial(
                    &mut frames,
                    &mut used,
                    cfg,
                    packed,
                    has_miso,
                    bit_count,
                    mosi_word,
                    miso_word,
                    packed_word,
                    t_word0,
                    t,
                );
                bit_count = 0;
                mosi_word = 0;
                miso_word = 0;
                packed_word = 0;
                cs_on = false;
                if !try_push_frame(
                    &mut frames,
                    &mut used,
                    BusFrame {
                        t_start: t,
                        t_end: t,
                        summary: "CS#".into(),
                        bytes: Vec::new(),
                    },
                ) {
                    truncated = true;
                }
            }
            Evt::Clk(t) => {
                if !cs_on {
                    continue;
                }
                if bit_count == 0 {
                    t_word0 = t;
                }
                if packed {
                    let chunk = sample_packed(bpc, &lane0, lane1.as_ref(), lane2.as_ref(), lane3.as_ref(), t);
                    shift_in(cfg, &mut packed_word, chunk, bpc, &mut bit_count);
                    if bit_count >= word_bits {
                        let bytes = word_bytes(packed_word, word_bits);
                        let summary = format!("DATA 0x{:0width$X}", packed_word, width = hex_digits(word_bits));
                        if !try_push_frame(
                            &mut frames,
                            &mut used,
                            BusFrame {
                                t_start: t_word0,
                                t_end: t,
                                summary,
                                bytes,
                            },
                        ) {
                            truncated = true;
                            break;
                        }
                        packed_word = 0;
                        bit_count = 0;
                    }
                } else {
                    let mosi_bit = u32::from(sample_lane(Some(&lane0), t));
                    let miso_bit = if has_miso {
                        u32::from(sample_lane(lane1.as_ref(), t))
                    } else {
                        0
                    };
                    if cfg.msb_first {
                        mosi_word = (mosi_word << 1) | mosi_bit;
                        if has_miso {
                            miso_word = (miso_word << 1) | miso_bit;
                        }
                    } else {
                        mosi_word |= mosi_bit << bit_count;
                        if has_miso {
                            miso_word |= miso_bit << bit_count;
                        }
                    }
                    bit_count = bit_count.saturating_add(1);
                    if bit_count >= word_bits {
                        let mut summary = format!(
                            "MOSI 0x{:0width$X}",
                            mosi_word,
                            width = hex_digits(word_bits)
                        );
                        let mut bytes = word_bytes(mosi_word, word_bits);
                        if has_miso {
                            summary.push_str(&format!(
                                " / MISO 0x{:0width$X}",
                                miso_word,
                                width = hex_digits(word_bits)
                            ));
                            bytes.extend(word_bytes(miso_word, word_bits));
                        }
                        if !try_push_frame(
                            &mut frames,
                            &mut used,
                            BusFrame {
                                t_start: t_word0,
                                t_end: t,
                                summary,
                                bytes,
                            },
                        ) {
                            truncated = true;
                            break;
                        }
                        mosi_word = 0;
                        miso_word = 0;
                        bit_count = 0;
                    }
                }
            }
        }
    }

    if bit_count > 0 {
        truncated |= !flush_partial(
            &mut frames,
            &mut used,
            cfg,
            packed,
            has_miso,
            bit_count,
            mosi_word,
            miso_word,
            packed_word,
            t_word0,
            clk.x.last().copied().unwrap_or(t_word0),
        );
    }

    let mut info = format!(
        "SPI {} Mode {} {}-bit {}: {} item(s)",
        cfg.wire.label(),
        cfg.mode.label(),
        word_bits,
        if cfg.msb_first { "MSB" } else { "LSB" },
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
            Some("No SPI words decoded — check CLK/data/CS and mode".into())
        } else {
            None
        },
        truncated,
    }
}

fn need(msg: &str) -> BusDecodeResult {
    BusDecodeResult {
        frames: Vec::new(),
        info: String::new(),
        error: Some(msg.into()),
        ..Default::default()
    }
}

fn evt_t(e: &Evt) -> f64 {
    match *e {
        Evt::CsOn(t) | Evt::CsOff(t) | Evt::Clk(t) => t,
    }
}

fn evt_pri(e: &Evt) -> u8 {
    match e {
        Evt::CsOn(_) => 0,
        Evt::Clk(_) => 1,
        Evt::CsOff(_) => 2,
    }
}

fn sample_lane(lane: Option<&Lane<'_>>, t: f64) -> bool {
    lane.and_then(|l| l.wave.before(t)).unwrap_or(false)
}

fn sample_packed(
    bpc: u8,
    io0: &Lane<'_>,
    io1: Option<&Lane<'_>>,
    io2: Option<&Lane<'_>>,
    io3: Option<&Lane<'_>>,
    t: f64,
) -> u32 {
    let b0 = u32::from(sample_lane(Some(io0), t));
    let b1 = u32::from(sample_lane(io1, t));
    let b2 = u32::from(sample_lane(io2, t));
    let b3 = u32::from(sample_lane(io3, t));
    match bpc {
        4 => (b3 << 3) | (b2 << 2) | (b1 << 1) | b0,
        2 => (b1 << 1) | b0,
        _ => b0,
    }
}

fn shift_in(cfg: &SpiConfig, word: &mut u32, chunk: u32, nbits: u8, bit_count: &mut u8) {
    let mask = if nbits >= 32 {
        u32::MAX
    } else {
        (1u32 << nbits) - 1
    };
    let chunk = chunk & mask;
    if cfg.msb_first {
        *word = (*word << nbits) | chunk;
    } else {
        *word |= chunk << *bit_count;
    }
    *bit_count = bit_count.saturating_add(nbits);
}

fn hex_digits(bits: u8) -> usize {
    ((bits as usize) + 3) / 4
}

fn word_bytes(word: u32, bits: u8) -> Vec<u8> {
    let nbytes = ((bits as usize) + 7) / 8;
    let n = nbytes.max(1);
    let mut out = vec![0u8; n];
    for i in 0..n {
        out[n - 1 - i] = ((word >> (8 * i)) & 0xFF) as u8;
    }
    out
}

fn flush_partial(
    frames: &mut Vec<BusFrame>,
    used: &mut usize,
    cfg: &SpiConfig,
    packed: bool,
    has_miso: bool,
    bit_count: u8,
    mosi_word: u32,
    miso_word: u32,
    packed_word: u32,
    t0: f64,
    t1: f64,
) -> bool {
    if bit_count == 0 {
        return true;
    }
    if packed {
        try_push_frame(
            frames,
            used,
            BusFrame {
                t_start: t0,
                t_end: t1,
                summary: format!("DATA 0x{packed_word:X} (partial {bit_count}b)"),
                bytes: vec![(packed_word & 0xFF) as u8],
            },
        )
    } else {
        let mut summary = format!("MOSI 0x{mosi_word:X} (partial {bit_count}b)");
        let mut bytes = vec![(mosi_word & 0xFF) as u8];
        if has_miso {
            summary.push_str(&format!(" / MISO 0x{miso_word:X}"));
            bytes.push((miso_word & 0xFF) as u8);
        }
        let _ = cfg;
        try_push_frame(
            frames,
            used,
            BusFrame {
                t_start: t0,
                t_end: t1,
                summary,
                bytes,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold(
        x: &mut Vec<f64>,
        clk: &mut Vec<f64>,
        lines: &mut [Vec<f64>],
        t: &mut f64,
        dt: f64,
        n: usize,
        clk_v: f64,
        vals: &[f64],
    ) {
        for _ in 0..n {
            x.push(*t);
            clk.push(clk_v);
            for (line, v) in lines.iter_mut().zip(vals.iter()) {
                line.push(*v);
            }
            *t += dt;
        }
    }

    fn traces(x: &[f64], ys: &[Vec<f64>], names: &[&str]) -> Vec<WaveformTrace> {
        ys.iter()
            .zip(names.iter())
            .map(|(y, name)| WaveformTrace {
                channel: (*name).into(),
                x: x.to_vec().into(),
                y: y.clone().into(),
                x_unit: "s".into(),
                y_unit: "V".into(),
            })
            .collect()
    }

    fn bit_volt(bit: bool) -> f64 {
        if bit { 3.3 } else { 0.0 }
    }

    /// Mode 0: idle CLK low, sample on rising. CS active low.
    fn synth_4wire(mosi: &[u8], miso: &[u8]) -> Vec<WaveformTrace> {
        let dt = 1e-7;
        let n = 3usize;
        let mut t = 0.0;
        let mut x = Vec::new();
        let mut clk = Vec::new();
        let mut lines = [Vec::new(), Vec::new(), Vec::new()];
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n * 2, 0.0, &[0.0, 0.0, 3.3]);
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[0.0, 0.0, 0.0]);
        for (mb, ib) in mosi.iter().zip(miso.iter()) {
            for i in (0..8).rev() {
                let mv = bit_volt((mb >> i) & 1 == 1);
                let iv = bit_volt((ib >> i) & 1 == 1);
                hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[mv, iv, 0.0]);
                hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 3.3, &[mv, iv, 0.0]);
                hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[mv, iv, 0.0]);
            }
        }
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[0.0, 0.0, 0.0]);
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n * 2, 0.0, &[0.0, 0.0, 3.3]);
        let [mosi_y, miso_y, cs_y] = lines;
        traces(&x, &[clk, mosi_y, miso_y, cs_y], &["CLK", "MOSI", "MISO", "CS"])
    }

    fn synth_dual(byte: u8) -> Vec<WaveformTrace> {
        let dt = 1e-7;
        let n = 3usize;
        let mut t = 0.0;
        let mut x = Vec::new();
        let mut clk = Vec::new();
        let mut lines = [Vec::new(), Vec::new(), Vec::new()];
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[0.0, 0.0, 0.0]);
        for k in 0..4 {
            let shift = 6 - 2 * k;
            let pair = (byte >> shift) & 0b11;
            let io0 = bit_volt(pair & 1 == 1);
            let io1 = bit_volt(pair & 2 == 2);
            hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[io0, io1, 0.0]);
            hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 3.3, &[io0, io1, 0.0]);
            hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[io0, io1, 0.0]);
        }
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[0.0, 0.0, 3.3]);
        let [io0, io1, cs] = lines;
        traces(&x, &[clk, io0, io1, cs], &["CLK", "IO0", "IO1", "CS"])
    }

    fn synth_quad(byte: u8) -> Vec<WaveformTrace> {
        let dt = 1e-7;
        let n = 3usize;
        let mut t = 0.0;
        let mut x = Vec::new();
        let mut clk = Vec::new();
        let mut lines = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[0.0, 0.0, 0.0, 0.0, 0.0]);
        for k in 0..2 {
            let nibble = if k == 0 { byte >> 4 } else { byte & 0x0F };
            let v = [
                bit_volt(nibble & 1 == 1),
                bit_volt(nibble & 2 == 2),
                bit_volt(nibble & 4 == 4),
                bit_volt(nibble & 8 == 8),
                0.0,
            ];
            hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &v);
            hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 3.3, &v);
            hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &v);
        }
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[0.0, 0.0, 0.0, 0.0, 3.3]);
        let [io0, io1, io2, io3, cs] = lines;
        traces(&x, &[clk, io0, io1, io2, io3, cs], &["CLK", "IO0", "IO1", "IO2", "IO3", "CS"])
    }

    #[test]
    fn four_wire_mode0_full_duplex() {
        let w = synth_4wire(&[0xA5], &[0x5A]);
        let r = decode_spi(
            &w[0],
            Some(&w[1]),
            Some(&w[2]),
            None,
            None,
            Some(&w[3]),
            None,
            &SpiConfig::default(),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        let texts: Vec<&str> = r.frames.iter().map(|f| f.summary.as_str()).collect();
        assert!(texts.contains(&"CS"), "{texts:?}");
        assert!(texts.iter().any(|s| s.contains("MOSI 0xA5") && s.contains("MISO 0x5A")), "{texts:?}");
        assert!(texts.contains(&"CS#"), "{texts:?}");
    }

    #[test]
    fn two_wire_mosi_only() {
        let w = synth_4wire(&[0x3C], &[0x00]);
        let mut cfg = SpiConfig::default();
        cfg.wire = SpiWire::TwoWire;
        let r = decode_spi(&w[0], Some(&w[1]), None, None, None, Some(&w[3]), None, &cfg);
        assert!(r.frames.iter().any(|f| f.summary.contains("MOSI 0x3C")), "{:?}", r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>());
        assert!(!r.frames.iter().any(|f| f.summary.contains("MISO")));
    }

    #[test]
    fn dual_packs_two_bits_per_clock() {
        let w = synth_dual(0xA5);
        let mut cfg = SpiConfig::default();
        cfg.wire = SpiWire::Dual;
        let r = decode_spi(&w[0], Some(&w[1]), Some(&w[2]), None, None, Some(&w[3]), None, &cfg);
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(
            r.frames.iter().any(|f| f.summary.contains("DATA 0xA5")),
            "{:?}",
            r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>()
        );
    }

    #[test]
    fn quad_packs_four_bits_per_clock() {
        let w = synth_quad(0xA5);
        let mut cfg = SpiConfig::default();
        cfg.wire = SpiWire::Quad;
        let r = decode_spi(
            &w[0],
            Some(&w[1]),
            Some(&w[2]),
            Some(&w[3]),
            Some(&w[4]),
            Some(&w[5]),
            None,
            &cfg,
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(
            r.frames.iter().any(|f| f.summary.contains("DATA 0xA5")),
            "{:?}",
            r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>()
        );
    }

    /// Mode 2: idle CLK high, sample on falling.
    fn synth_4wire_mode2(mosi: u8) -> Vec<WaveformTrace> {
        let dt = 1e-7;
        let n = 3usize;
        let mut t = 0.0;
        let mut x = Vec::new();
        let mut clk = Vec::new();
        let mut lines = [Vec::new(), Vec::new(), Vec::new()];
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n * 2, 3.3, &[0.0, 0.0, 3.3]);
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 3.3, &[0.0, 0.0, 0.0]);
        for i in (0..8).rev() {
            let mv = bit_volt((mosi >> i) & 1 == 1);
            hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 3.3, &[mv, 0.0, 0.0]);
            hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 0.0, &[mv, 0.0, 0.0]);
            hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 3.3, &[mv, 0.0, 0.0]);
        }
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n, 3.3, &[0.0, 0.0, 0.0]);
        hold(&mut x, &mut clk, &mut lines, &mut t, dt, n * 2, 3.3, &[0.0, 0.0, 3.3]);
        let [mosi_y, miso_y, cs_y] = lines;
        traces(&x, &[clk, mosi_y, miso_y, cs_y], &["CLK", "MOSI", "MISO", "CS"])
    }

    #[test]
    fn mode2_samples_falling_edge() {
        let w = synth_4wire_mode2(0xC3);
        let mut cfg = SpiConfig::default();
        cfg.mode = SpiMode::Mode2;
        let r = decode_spi(&w[0], Some(&w[1]), Some(&w[2]), None, None, Some(&w[3]), None, &cfg);
        assert!(
            r.frames.iter().any(|f| f.summary.contains("MOSI 0xC3")),
            "{:?}",
            r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>()
        );
    }

    #[test]
    fn lsb_first_reverses_bit_order() {
        let w = synth_4wire(&[0xB1], &[0x00]);
        let mut cfg = SpiConfig::default();
        cfg.msb_first = false;
        let r = decode_spi(&w[0], Some(&w[1]), None, None, None, Some(&w[3]), None, &cfg);
        // 0xB1 MSB-first on the wire, read LSB-first → bit reverse = 0x8D.
        assert!(
            r.frames.iter().any(|f| f.summary.contains("MOSI 0x8D")),
            "{:?}",
            r.frames.iter().map(|f| &f.summary).collect::<Vec<_>>()
        );
    }
}
