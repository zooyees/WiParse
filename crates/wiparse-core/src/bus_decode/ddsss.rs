//! DDSSS (Qi PRx→PTx DSSS ASK) demodulation from an analog power waveform.
//!
//! Offline MCU-style front end: one envelope sample per power cycle, differential
//! chips, sliding correlator, then Qi header/payload/checksum bytes.
//! Spec: Qi *DDSSS Communications* Draft 5 (2026-02-25).

use super::{
    try_push_frame, BusBitKind, BusBitMark, BusByteError, BusByteSpan, BusChipMark, BusDecodeResult,
    BusFrame, BusFrameError, MAX_DECODE_BYTES,
};
use crate::instrument::WaveformTrace;
use crate::protocol::decode::split_payload_checksum;
use crate::protocol::defs::{ask_packet, get_payload_len};

/// Spreading sequence selection. `Auto` tries A–D and keeps the best lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DdsssSequence {
    Auto,
    #[default]
    SeqA,
    SeqB,
    SeqC,
    SeqD,
}

impl DdsssSequence {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::SeqA => "SEQA",
            Self::SeqB => "SEQB",
            Self::SeqC => "SEQC",
            Self::SeqD => "SEQD",
        }
    }

    pub fn all_selectable() -> &'static [DdsssSequence] {
        &[
            Self::Auto,
            Self::SeqA,
            Self::SeqB,
            Self::SeqC,
            Self::SeqD,
        ]
    }

    pub fn parse_label(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "seqa" | "a" | "31" => Some(Self::SeqA),
            "seqb" | "b" | "15" => Some(Self::SeqB),
            "seqc" | "c" | "11" => Some(Self::SeqC),
            "seqd" | "d" | "7" => Some(Self::SeqD),
            _ => None,
        }
    }
}

/// Optional balancing / extension chip after the base sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DdsssExtension {
    Auto,
    #[default]
    Off,
    On,
}

impl DdsssExtension {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Off => "Off",
            Self::On => "On",
        }
    }

    pub fn all_selectable() -> &'static [DdsssExtension] {
        &[Self::Auto, Self::Off, Self::On]
    }

    pub fn parse_label(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "off" | "0" | "false" | "no" => Some(Self::Off),
            "on" | "1" | "true" | "yes" => Some(Self::On),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DdsssConfig {
    pub sequence: DdsssSequence,
    pub extension: DdsssExtension,
    /// Manual FOP (Hz). `None` = estimate from zero crossings.
    pub fop_hz: Option<f64>,
}

const FOP_MIN_HZ: f64 = 80_000.0;
const FOP_MAX_HZ: f64 = 2_000_000.0;
const MAX_CHIP_MARKS: usize = 65_536;
const PREAMBLE_BITS: u8 = 11;
const DQM_BITS: usize = 32;

/// Table 2 base sequences (leftmost chip first). Optional extension chip in `ext_chip`.
const SEQA: &str = "1111100011011101010000100101100";
const SEQB: &str = "111101011001000";
const SEQC: &str = "11100010010";
const SEQD: &str = "1110010";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeqId {
    A,
    B,
    C,
    D,
}

impl SeqId {
    fn all() -> &'static [SeqId] {
        &[SeqId::A, SeqId::B, SeqId::C, SeqId::D]
    }

    fn label(self) -> &'static str {
        match self {
            Self::A => "SEQA",
            Self::B => "SEQB",
            Self::C => "SEQC",
            Self::D => "SEQD",
        }
    }

    fn from_choice(choice: DdsssSequence) -> &'static [SeqId] {
        match choice {
            DdsssSequence::Auto => Self::all(),
            DdsssSequence::SeqA => &[SeqId::A],
            DdsssSequence::SeqB => &[SeqId::B],
            DdsssSequence::SeqC => &[SeqId::C],
            DdsssSequence::SeqD => &[SeqId::D],
        }
    }

    fn bits(self, extension: bool) -> Vec<bool> {
        let base = match self {
            Self::A => SEQA,
            Self::B => SEQB,
            Self::C => SEQC,
            Self::D => SEQD,
        };
        let mut v = parse_bits(base);
        if extension {
            v.push(self.ext_chip());
        }
        v
    }

    fn ext_chip(self) -> bool {
        // Table 2 optional balancing chip.
        matches!(self, Self::C)
    }

    /// Search high / search low / demod threshold (Table 4).
    fn thresholds(self, extension: bool) -> (u32, u32, u32) {
        match (self, extension) {
            (Self::A, false) => (22, 8, 15),
            (Self::A, true) => (23, 8, 15),
            (Self::B, _) => (12, 2, 7),
            (Self::C, false) => (9, 1, 5),
            (Self::C, true) => (10, 1, 5),
            (Self::D, _) => (5, 1, 3),
        }
    }
}

fn parse_bits(s: &str) -> Vec<bool> {
    s.chars().map(|c| c == '1').collect()
}

/// Fibonacci LFSR: width `n`, taps as bit indices (b0 is the output).
/// Starts with all bits ONE; new bit is inserted at the MSB after a right shift.
pub fn lfsr_msequence(width: u8, taps: &[u8]) -> Vec<bool> {
    let n = width as u32;
    let period = (1u32 << n) - 1;
    let mut state = (1u32 << n) - 1;
    let mut out = Vec::with_capacity(period as usize);
    for _ in 0..period {
        out.push((state & 1) != 0);
        let mut fb = false;
        for &t in taps {
            fb ^= ((state >> t) & 1) != 0;
        }
        state = (state >> 1) | (u32::from(fb) << (n - 1));
    }
    out
}

pub fn decode_ddsss(trace: &WaveformTrace, cfg: &DdsssConfig) -> BusDecodeResult {
    let n = trace.x.len().min(trace.y.len());
    if n < 32 {
        return BusDecodeResult {
            error: Some("Trace too short for DDSSS decode".into()),
            ..Default::default()
        };
    }

    let manual_fop = cfg.fop_hz.filter(|f| *f > 0.0);
    let estimated = if manual_fop.is_some() {
        None
    } else {
        estimate_fop(trace)
    };
    let fop = manual_fop
        .or(estimated)
        .filter(|f| *f >= FOP_MIN_HZ && *f <= FOP_MAX_HZ);
    let Some(fop) = fop else {
        let hint = estimated
            .map(|f| format!("estimated {f:.1} Hz (need {FOP_MIN_HZ:.0}–{FOP_MAX_HZ:.0})"))
            .unwrap_or_else(|| "could not estimate FOP from zero crossings".into());
        return BusDecodeResult {
            error: Some(format!(
                "FOP unavailable ({hint}); set a manual FOP in 85 kHz–1.78 MHz"
            )),
            ..Default::default()
        };
    };

    let (env, times) = cycle_envelope(trace, fop);
    if env.len() < 16 {
        return BusDecodeResult {
            error: Some("Too few power cycles for DDSSS decode".into()),
            info: format!("FOP={:.1} kHz cycles={}", fop / 1e3, env.len()),
            ..Default::default()
        };
    }
    let depth = modulation_depth(&env);
    let dead_frac = adaptive_dead_frac(depth);
    let mut cycle_chips = differential_chips(&env, dead_frac);
    if cycle_chips.len() < 8 {
        return BusDecodeResult {
            error: Some("Differential chip stream too short".into()),
            info: format!("FOP={:.1} kHz depth={:.2}%", fop / 1e3, depth * 100.0),
            ..Default::default()
        };
    }

    let mut best = search_attempts(&cycle_chips, &times, fop, cfg);
    let mut shallow_retry = false;
    if best.as_ref().map(|a| a.frames.is_empty()).unwrap_or(true) && dead_frac > 0.0 {
        cycle_chips = differential_chips(&env, 0.0);
        let retry = search_attempts(&cycle_chips, &times, fop, cfg);
        if retry
            .as_ref()
            .map(|a| match &best {
                None => true,
                Some(prev) => a.better_than(prev),
            })
            .unwrap_or(false)
        {
            best = retry;
            shallow_retry = true;
        }
    }

    let Some(best) = best else {
        return BusDecodeResult {
            info: format!("FOP={:.1} kHz", fop / 1e3),
            error: Some("DDSSS correlator produced no candidate".into()),
            ..Default::default()
        };
    };

    let nseq = best.seq.bits(best.extension).len();
    let fchip = fop / 2.0;
    let kbps = if nseq > 0 {
        fchip / nseq as f64 / 1e3
    } else {
        0.0
    };
    let dqm = if best.dqm.is_empty() {
        String::new()
    } else {
        format!(
            "  DQM={}",
            best.dqm
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let retry = if shallow_retry { "  shallow-retry" } else { "" };
    let cs_err = best
        .frames
        .iter()
        .filter(|f| f.error == BusFrameError::Checksum)
        .count();
    let p_err = best
        .frames
        .iter()
        .filter(|f| f.error == BusFrameError::Parity)
        .count();
    let f_err = best
        .frames
        .iter()
        .filter(|f| f.error == BusFrameError::Framing)
        .count();
    let chip_err = best.chips.iter().filter(|c| c.error).count();
    let err_info = if cs_err + p_err + f_err + chip_err == 0 {
        String::new()
    } else {
        format!("  cs_err={cs_err}  P_err={p_err}  F_err={f_err}  chip_err={chip_err}")
    };
    let info = format!(
        "FOP={:.1} kHz  fchip={:.1} kHz  {kbps:.1} kbps  {} Nseq={nseq} ext={}  phase={}  invert={}  depth={:.1}%  known={:.0}%  frames={}  chips={}  peak={:.1}{retry}{dqm}{err_info}",
        fop / 1e3,
        fchip / 1e3,
        best.seq.label(),
        if best.extension { "on" } else { "off" },
        if best.phase == 0 { "even" } else { "odd" },
        best.invert,
        depth * 100.0,
        best.known_frac * 100.0,
        best.frames.len(),
        best.chips.len(),
        best.best_metric,
    );

    BusDecodeResult {
        frames: best.frames,
        info,
        error: None,
        truncated: best.truncated,
        chips: best.chips,
        bits: best.bits,
        byte_spans: best.byte_spans,
    }
}

fn search_attempts(
    cycle_chips: &[Option<bool>],
    times: &[f64],
    fop: f64,
    cfg: &DdsssConfig,
) -> Option<Attempt> {
    let auto_seq = cfg.sequence == DdsssSequence::Auto;
    let auto_ext = cfg.extension == DdsssExtension::Auto;
    let seqs: Vec<SeqId> = SeqId::from_choice(cfg.sequence).to_vec();
    let exts: Vec<bool> = match cfg.extension {
        DdsssExtension::Auto => vec![false, true],
        DdsssExtension::Off => vec![false],
        DdsssExtension::On => vec![true],
    };

    let mut best: Option<Attempt> = None;
    'outer: for seq in seqs {
        for &ext in &exts {
            let mut got_ok = false;
            for phase in 0..2 {
                let attempt = demod_phase(cycle_chips, times, fop, seq, ext, phase);
                got_ok |= attempt
                    .frames
                    .iter()
                    .any(|f| f.error == BusFrameError::None);
                let take = match &best {
                    None => true,
                    Some(prev) => attempt.better_than(prev),
                };
                if take {
                    best = Some(attempt);
                }
            }
            if got_ok && (auto_seq || auto_ext) {
                break 'outer;
            }
        }
    }
    best
}

struct Attempt {
    frames: Vec<BusFrame>,
    truncated: bool,
    seq: SeqId,
    extension: bool,
    phase: usize,
    invert: bool,
    /// Mean |2c − Nseq| over locked bits (higher = sharper).
    best_metric: f64,
    dqm: Vec<u32>,
    chips: Vec<BusChipMark>,
    bits: Vec<BusBitMark>,
    byte_spans: Vec<BusByteSpan>,
    known_frac: f64,
}

impl Attempt {
    fn ok_frames(&self) -> usize {
        self.frames
            .iter()
            .filter(|f| f.error == BusFrameError::None)
            .count()
    }

    fn better_than(&self, other: &Attempt) -> bool {
        match self.ok_frames().cmp(&other.ok_frames()) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => match self.frames.len().cmp(&other.frames.len()) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Less => false,
                std::cmp::Ordering::Equal => self.best_metric > other.best_metric,
            },
        }
    }
}

struct ChipInst {
    value: Option<bool>,
    t_start: f64,
    t_end: f64,
}

fn demod_phase(
    cycle_chips: &[Option<bool>],
    cycle_times: &[f64],
    fop: f64,
    seq: SeqId,
    extension: bool,
    phase: usize,
) -> Attempt {
    let pattern = seq.bits(extension);
    let nseq = pattern.len();
    let (high, low, dmd) = seq.thresholds(extension);
    let chips = decimate_phase(cycle_chips, cycle_times, phase, fop);
    let empty = Attempt {
        frames: Vec::new(),
        truncated: false,
        seq,
        extension,
        phase,
        invert: false,
        best_metric: 0.0,
        dqm: Vec::new(),
        chips: Vec::new(),
        bits: Vec::new(),
        byte_spans: Vec::new(),
        known_frac: 0.0,
    };
    if chips.len() < nseq + 4 {
        return empty;
    }

    let mut frames = Vec::new();
    let mut used = 0usize;
    let mut truncated = false;
    let mut invert = false;
    let mut metrics: Vec<f64> = Vec::new();
    let mut dqm_acc: Vec<u32> = Vec::new();
    let mut i = 0usize;
    let mut locked = false;
    let mut preamble_run: u8 = 0;
    let mut preamble_last: Option<bool> = None;
    let mut polarity_fixed = false;
    let mut idle = false;
    let mut byte_bits: Vec<(bool, f64, f64, u32)> = Vec::new();
    let mut packet: Vec<u8> = Vec::new();
    let mut packet_start = 0.0_f64;
    let mut packet_bit_metrics: Vec<u32> = Vec::new();
    let mut packet_parity_err = false;
    let mut expected = 0usize;
    let mut byte_spans: Vec<BusByteSpan> = Vec::new();
    let mut bit_marks: Vec<BusBitMark> = Vec::new();
    let mut chip_marks: Vec<BusChipMark> = Vec::new();

    let abort_packet = |idle: &mut bool,
                        byte_bits: &mut Vec<(bool, f64, f64, u32)>,
                        packet: &mut Vec<u8>,
                        expected: &mut usize,
                        packet_bit_metrics: &mut Vec<u32>,
                        packet_parity_err: &mut bool| {
        *idle = true;
        byte_bits.clear();
        packet.clear();
        *expected = 0;
        packet_bit_metrics.clear();
        *packet_parity_err = false;
    };

    while i + nseq <= chips.len() {
        let (corr, known) = correlate(&chips, &pattern, i);
        let min_known = (nseq as u32 * 3 + 3) / 4;
        if !locked {
            if known >= min_known && (corr >= high || corr <= low) {
                locked = true;
                // Fall through and consume this window as the first bit.
            } else {
                i += 1;
                continue;
            }
        }

        let raw_one = corr >= dmd;
        let metric = (2.0 * f64::from(corr) - nseq as f64).abs();
        metrics.push(metric);
        let t_bit = chips.get(i).map(|c| c.t_start).unwrap_or(0.0);
        let t_end = chips
            .get(i + nseq - 1)
            .map(|c| c.t_end)
            .unwrap_or(t_bit);

        if !polarity_fixed {
            let bit = raw_one;
            match preamble_last {
                None => {
                    preamble_last = Some(bit);
                    preamble_run = 1;
                }
                Some(prev) if prev == bit => {
                    preamble_run = preamble_run.saturating_add(1);
                }
                Some(_) => {
                    preamble_last = Some(bit);
                    preamble_run = 1;
                }
            }
            if preamble_run >= PREAMBLE_BITS {
                invert = !preamble_last.unwrap_or(true);
                polarity_fixed = true;
                idle = true;
            }
            i += nseq;
            continue;
        }

        let bit = if invert { !raw_one } else { raw_one };

        if idle {
            if bit {
                i += nseq;
                continue;
            }
            idle = false;
            byte_bits.clear();
            packet.clear();
            expected = 0;
            packet_bit_metrics.clear();
            packet_parity_err = false;
            packet_start = t_bit;
        }

        byte_bits.push((bit, t_bit, t_end, corr));
        packet_bit_metrics.push(corr);
        push_bit_chips(
            &mut chip_marks,
            &chips,
            i,
            nseq,
            &pattern,
            raw_one,
            invert,
        );

        if byte_bits.len() == 11 {
            let assembled = assemble_byte(&byte_bits);
            let span_start = byte_bits.first().map(|b| b.1).unwrap_or(t_bit);
            let byte_end = byte_bits.last().map(|b| b.2).unwrap_or(packet_start);
            bit_marks.extend(bit_marks_from_byte(
                &byte_bits,
                !assembled.parity_ok,
                !assembled.framing_ok,
            ));
            if !assembled.framing_ok {
                byte_spans.push(BusByteSpan {
                    t_start: span_start,
                    t_end: byte_end,
                    byte: assembled.byte,
                    error: BusByteError::Framing,
                });
                if !packet.is_empty() {
                    let name = ask_packet(packet[0]).map(|m| m.name).unwrap_or("UNK");
                    let frame = BusFrame {
                        t_start: packet_start,
                        t_end: byte_end,
                        summary: name.to_string(),
                        bytes: packet.clone(),
                        error: BusFrameError::Framing,
                    };
                    if !try_push_frame(&mut frames, &mut used, frame) {
                        truncated = true;
                        break;
                    }
                }
                abort_packet(
                    &mut idle,
                    &mut byte_bits,
                    &mut packet,
                    &mut expected,
                    &mut packet_bit_metrics,
                    &mut packet_parity_err,
                );
                i += nseq;
                continue;
            }
            let byte_err = if assembled.parity_ok {
                BusByteError::None
            } else {
                packet_parity_err = true;
                BusByteError::Parity
            };
            byte_spans.push(BusByteSpan {
                t_start: span_start,
                t_end: byte_end,
                byte: assembled.byte,
                error: byte_err,
            });
            if packet.is_empty() {
                expected = 1 + get_payload_len(assembled.byte) as usize + 1;
            }
            packet.push(assembled.byte);
            byte_bits.clear();
            if expected > 0 && packet.len() >= expected {
                let header = packet[0];
                let (_payload, _cs, ok) = split_payload_checksum(header, &packet[1..]);
                let name = ask_packet(header).map(|m| m.name).unwrap_or("UNK");
                let checksum_ok = ok == Some(true);
                if checksum_ok {
                    dqm_acc.push(dqm_from_corrs(&packet_bit_metrics, nseq));
                } else if let Some(last) = byte_spans.last_mut() {
                    if last.error == BusByteError::None {
                        last.error = BusByteError::Checksum;
                    }
                }
                let error = if !checksum_ok {
                    BusFrameError::Checksum
                } else if packet_parity_err {
                    BusFrameError::Parity
                } else {
                    BusFrameError::None
                };
                let frame = BusFrame {
                    t_start: packet_start,
                    t_end: byte_end,
                    summary: name.to_string(),
                    bytes: packet.clone(),
                    error,
                };
                if !try_push_frame(&mut frames, &mut used, frame) {
                    truncated = true;
                    break;
                }
                abort_packet(
                    &mut idle,
                    &mut byte_bits,
                    &mut packet,
                    &mut expected,
                    &mut packet_bit_metrics,
                    &mut packet_parity_err,
                );
            }
        }

        i += nseq;
        if used >= MAX_DECODE_BYTES {
            truncated = true;
            break;
        }
    }

    let best_metric = if metrics.is_empty() {
        0.0
    } else {
        metrics.iter().sum::<f64>() / metrics.len() as f64
    };
    let known_frac = known_fraction(&chips);

    Attempt {
        frames,
        truncated,
        seq,
        extension,
        phase,
        invert,
        best_metric,
        dqm: dqm_acc,
        chips: chip_marks,
        bits: bit_marks,
        byte_spans,
        known_frac,
    }
}

fn dqm_from_corrs(corrs: &[u32], nseq: usize) -> u32 {
    corrs
        .iter()
        .take(DQM_BITS)
        .map(|&c| (2 * c as i32 - nseq as i32).unsigned_abs())
        .sum()
}

struct AssembledByte {
    byte: u8,
    parity_ok: bool,
    framing_ok: bool,
}

fn assemble_byte(bits: &[(bool, f64, f64, u32)]) -> AssembledByte {
    let start_ok = bits.len() == 11 && !bits[0].0;
    let stop_ok = bits.len() == 11 && bits[10].0;
    let mut byte = 0u8;
    let mut ones = 0u32;
    if bits.len() >= 9 {
        for (k, (bit, _, _, _)) in bits.iter().enumerate().skip(1).take(8) {
            if *bit {
                byte |= 1 << (k - 1);
                ones += 1;
            }
        }
    }
    let parity = bits.get(9).map(|b| b.0).unwrap_or(false);
    if parity {
        ones += 1;
    }
    AssembledByte {
        byte,
        parity_ok: ones % 2 == 1,
        framing_ok: start_ok && stop_ok,
    }
}

fn push_bit_chips(
    out: &mut Vec<BusChipMark>,
    chips: &[ChipInst],
    start: usize,
    nseq: usize,
    pattern: &[bool],
    raw_one: bool,
    invert: bool,
) {
    for k in 0..nseq {
        if out.len() >= MAX_CHIP_MARKS {
            break;
        }
        let Some(chip) = chips.get(start + k) else {
            break;
        };
        let expected_raw = if raw_one { pattern[k] } else { !pattern[k] };
        let error = match chip.value {
            Some(v) => v != expected_raw,
            None => true,
        };
        out.push(BusChipMark {
            t_start: chip.t_start,
            t_end: chip.t_end,
            one: chip.value.map(|b| if invert { !b } else { b }),
            error,
        });
    }
}

fn known_fraction(chips: &[ChipInst]) -> f64 {
    if chips.is_empty() {
        return 0.0;
    }
    let known = chips.iter().filter(|c| c.value.is_some()).count();
    known as f64 / chips.len() as f64
}

fn bit_marks_from_byte(
    bits: &[(bool, f64, f64, u32)],
    parity_err: bool,
    framing_err: bool,
) -> Vec<BusBitMark> {
    bits.iter()
        .enumerate()
        .map(|(k, (one, t0, t1, _))| {
            let kind = match k {
                0 => BusBitKind::Start,
                1..=8 => BusBitKind::Data {
                    index: (k - 1) as u8,
                },
                9 => BusBitKind::Parity,
                _ => BusBitKind::Stop,
            };
            let error = match kind {
                BusBitKind::Parity => parity_err,
                BusBitKind::Start | BusBitKind::Stop => framing_err,
                BusBitKind::Data { .. } => false,
            };
            BusBitMark {
                t_start: *t0,
                t_end: *t1,
                one: *one,
                kind,
                error,
            }
        })
        .collect()
}

fn correlate(chips: &[ChipInst], seq: &[bool], start: usize) -> (u32, u32) {
    let mut matches = 0u32;
    let mut known = 0u32;
    for (k, want) in seq.iter().enumerate() {
        match chips.get(start + k).and_then(|c| c.value) {
            Some(bit) => {
                known += 1;
                if bit == *want {
                    matches += 1;
                }
            }
            None => {}
        }
    }
    (matches, known)
}

fn decimate_phase(
    cycle_chips: &[Option<bool>],
    cycle_times: &[f64],
    phase: usize,
    fop: f64,
) -> Vec<ChipInst> {
    let period = if fop > 0.0 { 1.0 / fop } else { 0.0 };
    let mut out = Vec::new();
    let mut i = phase;
    while i < cycle_chips.len() {
        let mid_a = cycle_times.get(i).copied().unwrap_or(0.0);
        let mid_b = cycle_times.get(i + 1).copied().unwrap_or(mid_a + period);
        let half = {
            let span = (mid_b - mid_a).abs();
            if span > 1e-18 {
                span * 0.5
            } else {
                period * 0.5
            }
        };
        out.push(ChipInst {
            value: cycle_chips[i],
            t_start: mid_a - half,
            t_end: mid_b + half,
        });
        i += 2;
    }
    out
}

pub(crate) fn estimate_fop(trace: &WaveformTrace) -> Option<f64> {
    let n = trace.x.len().min(trace.y.len());
    if n < 16 {
        return None;
    }
    // A few dozen cycles are enough; skip the rest of a multi-million-point ISF.
    let n = n.min(80_000);
    let mean = trace.y[..n].iter().sum::<f64>() / n as f64;
    let mut last_rise: Option<f64> = None;
    let mut periods = Vec::new();
    let mut prev = trace.y[0] - mean;
    for i in 1..n {
        let cur = trace.y[i] - mean;
        if prev < 0.0 && cur >= 0.0 {
            let t = trace.x[i];
            if let Some(t0) = last_rise {
                let p = t - t0;
                if p > 0.0 {
                    periods.push(p);
                }
            }
            last_rise = Some(t);
            if periods.len() >= 240 {
                break;
            }
        }
        prev = cur;
    }
    let period = median(&mut periods)?;
    if period <= 0.0 {
        return None;
    }
    let fop = 1.0 / period;
    if (FOP_MIN_HZ..=FOP_MAX_HZ).contains(&fop) {
        Some(fop)
    } else {
        None
    }
}

fn rising_zero_times(trace: &WaveformTrace) -> Vec<f64> {
    let n = trace.x.len().min(trace.y.len());
    if n < 8 {
        return Vec::new();
    }
    let mean = trace.y[..n].iter().sum::<f64>() / n as f64;
    let mut out = Vec::new();
    let mut prev = trace.y[0] - mean;
    for i in 1..n {
        let cur = trace.y[i] - mean;
        if prev < 0.0 && cur >= 0.0 {
            let dy = cur - prev;
            let frac = if dy.abs() > 1e-30 { -prev / dy } else { 0.0 };
            let t0 = trace.x[i - 1];
            out.push(t0 + frac.clamp(0.0, 1.0) * (trace.x[i] - t0));
            if out.len() >= 500_000 {
                break;
            }
        }
        prev = cur;
    }
    out
}

fn cycle_envelope_from_zc(trace: &WaveformTrace, fop_hz: f64) -> Option<(Vec<f64>, Vec<f64>)> {
    let zc = rising_zero_times(trace);
    if zc.len() < 16 {
        return None;
    }
    let period = 1.0 / fop_hz;
    let n = trace.x.len().min(trace.y.len());
    let mut env = Vec::with_capacity(zc.len().saturating_sub(1));
    let mut times = Vec::with_capacity(zc.len().saturating_sub(1));
    let mut i = 0usize;
    for w in zc.windows(2) {
        let p = w[1] - w[0];
        if p < period * 0.5 || p > period * 1.6 {
            continue;
        }
        while i < n && trace.x[i] < w[0] {
            i += 1;
        }
        let mut sum = 0.0;
        let mut cnt = 0usize;
        let mut j = i;
        while j < n && trace.x[j] < w[1] {
            let y = trace.y[j];
            sum += y * y;
            cnt += 1;
            j += 1;
        }
        if cnt > 0 {
            env.push(sum / cnt as f64);
            times.push(0.5 * (w[0] + w[1]));
        }
        i = j;
    }
    if env.len() >= 16 {
        Some((env, times))
    } else {
        None
    }
}

pub(crate) fn cycle_envelope(trace: &WaveformTrace, fop_hz: f64) -> (Vec<f64>, Vec<f64>) {
    let n = trace.x.len().min(trace.y.len());
    if n == 0 || fop_hz <= 0.0 {
        return (Vec::new(), Vec::new());
    }
    if let Some(aligned) = cycle_envelope_from_zc(trace, fop_hz) {
        return aligned;
    }
    let period = 1.0 / fop_hz;
    let t0 = trace.x[0];
    let t_end = trace.x[n - 1];
    if t_end <= t0 {
        return (Vec::new(), Vec::new());
    }
    let n_cycles = ((t_end - t0) / period).floor() as usize;
    if n_cycles == 0 {
        return (Vec::new(), Vec::new());
    }
    if let Some(dt) = uniform_dt(&trace.x[..n]) {
        return cycle_envelope_uniform(&trace.y[..n], t0, dt, period, n_cycles);
    }
    let mut env = Vec::with_capacity(n_cycles);
    let mut times = Vec::with_capacity(n_cycles);
    let mut i = 0usize;
    for k in 0..n_cycles {
        let w0 = t0 + k as f64 * period;
        let w1 = w0 + period;
        while i < n && trace.x[i] < w0 {
            i += 1;
        }
        let mut sum = 0.0;
        let mut cnt = 0usize;
        let mut j = i;
        while j < n && trace.x[j] < w1 {
            let y = trace.y[j];
            sum += y * y;
            cnt += 1;
            j += 1;
        }
        if cnt == 0 {
            continue;
        }
        env.push(sum / cnt as f64);
        times.push(w0 + 0.5 * period);
        i = j;
    }
    (env, times)
}

fn uniform_dt(x: &[f64]) -> Option<f64> {
    if x.len() < 3 {
        return None;
    }
    let dt = (x[x.len() - 1] - x[0]) / (x.len() - 1) as f64;
    if dt <= 0.0 {
        return None;
    }
    let mid = x.len() / 2;
    let expected = x[0] + mid as f64 * dt;
    if (x[mid] - expected).abs() > dt * 0.08 {
        return None;
    }
    Some(dt)
}

fn cycle_envelope_uniform(
    y: &[f64],
    t0: f64,
    dt: f64,
    period: f64,
    n_cycles: usize,
) -> (Vec<f64>, Vec<f64>) {
    let spc = period / dt;
    if spc < 1.5 {
        return (Vec::new(), Vec::new());
    }
    let mut env = Vec::with_capacity(n_cycles);
    let mut times = Vec::with_capacity(n_cycles);
    let n = y.len();
    for k in 0..n_cycles {
        let start = ((k as f64) * spc).floor() as usize;
        let end = (((k as f64 + 1.0) * spc).floor() as usize).min(n);
        if end <= start {
            continue;
        }
        let mut sum = 0.0;
        for &v in &y[start..end] {
            sum += v * v;
        }
        env.push(sum / (end - start) as f64);
        times.push(t0 + (k as f64 + 0.5) * period);
    }
    (env, times)
}

fn modulation_depth(env: &[f64]) -> f64 {
    if env.len() < 4 {
        return 0.0;
    }
    let mean = env.iter().sum::<f64>() / env.len() as f64;
    if mean <= 1e-30 {
        return 0.0;
    }
    let var = env.iter().map(|v| {
        let d = *v - mean;
        d * d
    }).sum::<f64>() / env.len() as f64;
    var.sqrt() / mean
}

fn adaptive_dead_frac(depth: f64) -> f64 {
    if depth < 0.012 {
        0.04
    } else if depth < 0.04 {
        0.08
    } else {
        0.15
    }
}

pub(crate) fn differential_chips(env: &[f64], dead_frac: f64) -> Vec<Option<bool>> {
    if env.len() < 2 {
        return Vec::new();
    }
    let mut steps: Vec<f64> = env.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    let med = median(&mut steps).unwrap_or(0.0);
    let dead = if dead_frac <= 0.0 {
        0.0
    } else {
        (med * dead_frac).max(1e-18)
    };
    env.windows(2)
        .map(|w| {
            let d = w[1] - w[0];
            if dead > 0.0 && d.abs() < dead {
                None
            } else {
                Some(d > 0.0)
            }
        })
        .collect()
}

/// Build a Qi ASK frame: header + payload + XOR checksum.
pub fn qi_ask_frame(header: u8, payload: &[u8]) -> Vec<u8> {
    let mut xor = header;
    for b in payload {
        xor ^= *b;
    }
    let mut out = Vec::with_capacity(payload.len() + 2);
    out.push(header);
    out.extend_from_slice(payload);
    out.push(xor);
    out
}

const SYNTH_A_LO: f64 = 0.70;
const SYNTH_A_HI: f64 = 1.00;

/// Parameters for a synthetic VCTX/ILTX DDSSS capture.
#[derive(Debug, Clone)]
pub struct DdsssSynthRequest {
    pub fop_hz: f64,
    pub samples_per_cycle: usize,
    pub sequence: DdsssSequence,
    pub extension: bool,
    pub invert: bool,
    pub idle_preamble_bits: usize,
    pub idle_gap_bits: usize,
    pub packets: Vec<Vec<u8>>,
    pub channel: String,
    pub amp_lo: f64,
    pub amp_hi: f64,
    /// Flip this many chips after spreading (skip idle preamble). Mild enough
    /// that SEQA still locks; DQM drops a little.
    pub chip_errors: usize,
    /// Packet indices whose checksum byte is XOR'd with 1 (decoded as `NAME!`).
    pub checksum_errors: Vec<usize>,
    /// Packet indices whose header parity bit is flipped (decoded as `NAME P!`).
    pub parity_errors: Vec<usize>,
}

impl Default for DdsssSynthRequest {
    fn default() -> Self {
        Self {
            fop_hz: 128_000.0,
            samples_per_cycle: 8,
            sequence: DdsssSequence::SeqA,
            extension: false,
            invert: false,
            idle_preamble_bits: 14,
            idle_gap_bits: 12,
            packets: Vec::new(),
            channel: "VCTX".into(),
            amp_lo: SYNTH_A_LO,
            amp_hi: SYNTH_A_HI,
            chip_errors: 0,
            checksum_errors: Vec::new(),
            parity_errors: Vec::new(),
        }
    }
}

/// Synthesize a power-carrier waveform carrying one or more DDSSS Qi packets.
pub fn synthesize_ddsss(req: &DdsssSynthRequest) -> Result<WaveformTrace, String> {
    let seq = match req.sequence {
        DdsssSequence::Auto => {
            return Err("synthesize_ddsss requires a concrete sequence, not Auto".into());
        }
        DdsssSequence::SeqA => SeqId::A,
        DdsssSequence::SeqB => SeqId::B,
        DdsssSequence::SeqC => SeqId::C,
        DdsssSequence::SeqD => SeqId::D,
    };
    if req.fop_hz < FOP_MIN_HZ || req.fop_hz > FOP_MAX_HZ {
        return Err(format!(
            "fop_hz {} outside {}–{}",
            req.fop_hz, FOP_MIN_HZ, FOP_MAX_HZ
        ));
    }
    if req.samples_per_cycle < 4 {
        return Err("samples_per_cycle must be >= 4".into());
    }
    if req.packets.is_empty() {
        return Err("no packets".into());
    }
    let pattern = seq.bits(req.extension);
    let mut packets = req.packets.clone();
    for &idx in &req.checksum_errors {
        if let Some(pkt) = packets.get_mut(idx) {
            if let Some(cs) = pkt.last_mut() {
                *cs ^= 0x01;
            }
        }
    }
    let mut bits = vec![true; req.idle_preamble_bits.max(11)];
    for (i, pkt) in packets.iter().enumerate() {
        if i > 0 {
            bits.extend(std::iter::repeat(true).take(req.idle_gap_bits.max(11)));
        }
        for &byte in pkt {
            bits.extend(qi_byte_bits(byte));
        }
    }
    bits.extend(std::iter::repeat(true).take(req.idle_gap_bits.max(11)));
    inject_parity_errors(
        &mut bits,
        &packets,
        req.idle_preamble_bits.max(11),
        req.idle_gap_bits.max(11),
        &req.parity_errors,
    );
    let mut chips = data_bits_to_chips(&bits, &pattern);
    inject_chip_errors(
        &mut chips,
        &packets,
        req.idle_preamble_bits.max(11),
        req.idle_gap_bits.max(11),
        pattern.len(),
        req.chip_errors,
    );
    let amps = chips_to_amps(&chips, req.invert, req.amp_lo, req.amp_hi);
    Ok(amps_to_trace(
        &amps,
        req.fop_hz,
        req.samples_per_cycle,
        &req.channel,
    ))
}

fn qi_byte_bits(byte: u8) -> Vec<bool> {
    let mut v = vec![false];
    let mut ones = 0u32;
    for i in 0..8 {
        let b = (byte >> i) & 1 == 1;
        if b {
            ones += 1;
        }
        v.push(b);
    }
    v.push(ones % 2 == 0);
    v.push(true);
    v
}

fn inject_parity_errors(
    bits: &mut [bool],
    packets: &[Vec<u8>],
    preamble: usize,
    gap: usize,
    which: &[usize],
) {
    if which.is_empty() {
        return;
    }
    let mut offset = preamble;
    for (i, pkt) in packets.iter().enumerate() {
        if i > 0 {
            offset += gap;
        }
        if which.contains(&i) {
            let parity_i = offset + 9;
            if let Some(bit) = bits.get_mut(parity_i) {
                *bit = !*bit;
            }
        }
        offset += pkt.len() * 11;
    }
}

fn inject_chip_errors(
    chips: &mut [bool],
    packets: &[Vec<u8>],
    preamble_bits: usize,
    gap_bits: usize,
    nseq: usize,
    n: usize,
) {
    if n == 0 || nseq == 0 || chips.is_empty() {
        return;
    }
    let mut bit_starts = Vec::new();
    let mut bit_i = preamble_bits;
    for (pi, pkt) in packets.iter().enumerate() {
        if pi > 0 {
            bit_i += gap_bits;
        }
        for _ in 0..(pkt.len() * 11) {
            bit_starts.push(bit_i * nseq);
            bit_i += 1;
        }
    }
    if bit_starts.is_empty() {
        return;
    }
    // Table 4 SEQA demod threshold is 15/31; keep ≤2 flips per bit so the bit still decodes.
    let mut flipped_in_bit = vec![0u8; bit_starts.len()];
    let mut done = 0usize;
    let mut k = 0usize;
    while done < n && k < n.saturating_mul(16).max(16) {
        let bi = k.wrapping_mul(7919).wrapping_add(101) % bit_starts.len();
        k += 1;
        if flipped_in_bit[bi] >= 2 {
            continue;
        }
        let off = k.wrapping_mul(13).wrapping_add(3) % nseq;
        let idx = bit_starts[bi] + off;
        if let Some(c) = chips.get_mut(idx) {
            *c = !*c;
            flipped_in_bit[bi] += 1;
            done += 1;
        }
    }
}

fn data_bits_to_chips(bits: &[bool], seq: &[bool]) -> Vec<bool> {
    let mut chips = Vec::with_capacity(bits.len() * seq.len());
    for bit in bits {
        if *bit {
            chips.extend_from_slice(seq);
        } else {
            chips.extend(seq.iter().map(|c| !c));
        }
    }
    chips
}

fn chips_to_amps(chips: &[bool], invert: bool, amp_lo: f64, amp_hi: f64) -> Vec<f64> {
    let mut amps = Vec::with_capacity(chips.len() * 2);
    for chip in chips {
        let one = if invert { !*chip } else { *chip };
        if one {
            amps.push(amp_lo);
            amps.push(amp_hi);
        } else {
            amps.push(amp_hi);
            amps.push(amp_lo);
        }
    }
    amps
}

fn amps_to_trace(amps: &[f64], fop: f64, spc: usize, channel: &str) -> WaveformTrace {
    let dt = 1.0 / (fop * spc as f64);
    let mut x = Vec::with_capacity(amps.len() * spc);
    let mut y = Vec::with_capacity(amps.len() * spc);
    let mut t = 0.0;
    for &amp in amps {
        for i in 0..spc {
            x.push(t);
            y.push(amp * (2.0 * std::f64::consts::PI * i as f64 / spc as f64).sin());
            t += dt;
        }
    }
    WaveformTrace {
        channel: channel.into(),
        x: x.into(),
        y: y.into(),
        x_unit: "s".into(),
        y_unit: "V".into(),
    }
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        Some(v[n / 2])
    } else {
        Some(0.5 * (v[n / 2 - 1] + v[n / 2]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const FOP: f64 = 128_000.0;
    const SPC: usize = 8;
    const A_LO: f64 = 0.70;
    const A_HI: f64 = 1.00;

    #[test]
    fn lfsr_seqa_matches_table2() {
        let bits = lfsr_msequence(5, &[2, 0]);
        let got: String = bits.iter().map(|b| if *b { '1' } else { '0' }).collect();
        assert_eq!(got, SEQA);
        assert_eq!(bits.len(), 31);
    }

    #[test]
    fn lfsr_seqb_matches_table2() {
        let bits = lfsr_msequence(4, &[3, 0]);
        let got: String = bits.iter().map(|b| if *b { '1' } else { '0' }).collect();
        assert_eq!(got, SEQB);
        assert_eq!(bits.len(), 15);
    }

    #[test]
    fn table2_optional_chips() {
        assert!(!SeqId::A.ext_chip());
        assert!(!SeqId::B.ext_chip());
        assert!(SeqId::C.ext_chip());
        assert!(!SeqId::D.ext_chip());
        assert_eq!(SeqId::A.bits(true).len(), 32);
        assert_eq!(SeqId::C.bits(true).last().copied(), Some(true));
    }

    #[test]
    fn table4_thresholds() {
        assert_eq!(SeqId::A.thresholds(false), (22, 8, 15));
        assert_eq!(SeqId::A.thresholds(true), (23, 8, 15));
        assert_eq!(SeqId::B.thresholds(false), (12, 2, 7));
        assert_eq!(SeqId::C.thresholds(false), (9, 1, 5));
        assert_eq!(SeqId::C.thresholds(true), (10, 1, 5));
        assert_eq!(SeqId::D.thresholds(false), (5, 1, 3));
    }

    #[test]
    fn envelope_and_diff_follow_amplitude_steps() {
        // Two cycles low, two high → one step-up chip on the even phase.
        let amps = [A_LO, A_LO, A_HI, A_HI, A_LO, A_LO];
        let trace = super::amps_to_trace(&amps, FOP, SPC, "VCTX");
        let fop = estimate_fop(&trace).expect("fop");
        assert!((fop - FOP).abs() / FOP < 0.02, "fop={fop}");
        let (env, _) = cycle_envelope(&trace, FOP);
        assert!(env.len() >= 5, "env={}", env.len());
        let chips = differential_chips(&env, 0.15);
        // env[1]-env[0] ≈ 0 (both low), env[2]-env[1] step up, env[3]-env[2] ≈ 0,
        // env[4]-env[3] step down.
        assert!(chips.len() >= 4);
        assert_eq!(chips[1], Some(true), "{chips:?}");
        assert_eq!(chips[3], Some(false), "{chips:?}");
    }

    #[test]
    fn seqa_idle_peaks_every_62_cycles() {
        let seq = SeqId::A.bits(false);
        let nseq = seq.len();
        let data_bits = vec![true; 16];
        let chips = data_bits_to_chips(&data_bits, &seq);
        let amps = chips_to_amps(&chips, false, A_LO, A_HI);
        let cycle_chips = amps_to_cycle_chips(&amps);
        let high = 22u32;
        let mut peaks = Vec::new();
        let window = 2 * nseq;
        for i in 0..cycle_chips.len().saturating_sub(window) {
            let c = cycle_phase_corr(&cycle_chips, i, 0, &seq);
            if c >= high {
                if peaks.last().map(|p| i >= p + 40).unwrap_or(true) {
                    peaks.push(i);
                }
            }
        }
        assert!(peaks.len() >= 3, "peaks={peaks:?}");
        let spacings: Vec<usize> = peaks.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            spacings.iter().all(|&s| s == 62),
            "spacings={spacings:?} peaks={peaks:?}"
        );
    }

    #[test]
    fn decodes_ce_packet_seqa() {
        let trace = synth_qi_packet(SeqId::A, false, &[0x03, 0x00, 0x03], false);
        let r = decode_ddsss(
            &trace,
            &DdsssConfig {
                sequence: DdsssSequence::SeqA,
                extension: DdsssExtension::Off,
                fop_hz: Some(FOP),
            },
        );
        assert!(r.error.is_none(), "{r:?}");
        assert_eq!(r.frames.len(), 1, "info={} frames={:?}", r.info, r.frames);
        assert_eq!(r.frames[0].bytes, vec![0x03, 0x00, 0x03]);
        assert_eq!(r.frames[0].summary, "CE");
        assert_eq!(r.byte_spans.len(), 3, "bytes={:?}", r.byte_spans);
        assert_eq!(r.bits.len(), 33, "bits={}", r.bits.len());
        assert!(!r.chips.is_empty(), "chips empty info={}", r.info);
        assert!(r.frames[0].t_end > r.frames[0].t_start);
        assert!(r.info.contains("fchip="), "info={}", r.info);
        assert!(r.info.contains("Nseq="), "info={}", r.info);
        assert!(r.info.contains("DQM="), "info={}", r.info);
    }

    #[test]
    fn chip_windows_span_two_fop_cycles_and_nest_in_bits() {
        let trace = synth_qi_packet(SeqId::A, false, &[0x03, 0x00, 0x03], false);
        let r = decode_ddsss(
            &trace,
            &DdsssConfig {
                sequence: DdsssSequence::SeqA,
                extension: DdsssExtension::Off,
                fop_hz: Some(FOP),
            },
        );
        let chip_dt = 2.0 / FOP;
        assert!(!r.chips.is_empty());
        for c in &r.chips {
            let w = c.t_end - c.t_start;
            assert!(
                (w - chip_dt).abs() / chip_dt < 0.35,
                "chip width {w} vs {chip_dt}"
            );
        }
        for w in r.chips.windows(2) {
            assert!(w[1].t_start >= w[0].t_start - 1e-12);
            let gap = w[1].t_start - w[0].t_end;
            assert!(
                gap.abs() < chip_dt * 0.4,
                "chip gap {gap} dt={chip_dt} a=({},{}) b=({},{})",
                w[0].t_start,
                w[0].t_end,
                w[1].t_start,
                w[1].t_end
            );
        }
        let nseq = SEQA.len();
        for bit in &r.bits {
            let n = r
                .chips
                .iter()
                .filter(|c| c.t_start + 1e-12 >= bit.t_start && c.t_end <= bit.t_end + 1e-12)
                .count();
            assert!(
                n >= nseq.saturating_sub(2) && n <= nseq + 2,
                "bit [{}, {}] covers {n} chips, want ~{nseq}",
                bit.t_start,
                bit.t_end
            );
        }
        for b in &r.byte_spans {
            let nbits = r
                .bits
                .iter()
                .filter(|bit| bit.t_start + 1e-12 >= b.t_start && bit.t_end <= b.t_end + 1e-12)
                .count();
            assert_eq!(nbits, 11, "byte 0x{:02X} bits={nbits}", b.byte);
        }
    }

    #[test]
    fn inverted_polarity_still_decodes_ce() {
        let trace = synth_qi_packet(SeqId::A, false, &[0x03, 0x00, 0x03], true);
        let r = decode_ddsss(
            &trace,
            &DdsssConfig {
                sequence: DdsssSequence::SeqA,
                extension: DdsssExtension::Off,
                fop_hz: Some(FOP),
            },
        );
        assert_eq!(
            r.frames.first().map(|f| f.bytes.as_slice()),
            Some([0x03, 0x00, 0x03].as_slice()),
            "info={} err={:?}",
            r.info,
            r.error
        );
        assert!(r.info.contains("invert=true"), "info={}", r.info);
    }

    #[test]
    fn odd_phase_offset_still_locks() {
        // One extra power cycle at the front shifts true chips onto the odd phase.
        let mut amps = vec![A_HI];
        amps.extend(packet_amps(SeqId::A, false, &[0x03, 0x00, 0x03], false));
        let trace = super::amps_to_trace(&amps, FOP, SPC, "VCTX");
        let r = decode_ddsss(
            &trace,
            &DdsssConfig {
                sequence: DdsssSequence::SeqA,
                extension: DdsssExtension::Off,
                fop_hz: Some(FOP),
            },
        );
        assert_eq!(
            r.frames.first().map(|f| f.bytes.as_slice()),
            Some([0x03, 0x00, 0x03].as_slice()),
            "info={} err={:?}",
            r.info,
            r.error
        );
    }

    #[test]
    fn auto_picks_seqa_vs_seqd() {
        let a = synth_qi_packet(SeqId::A, false, &[0x03, 0x00, 0x03], false);
        let d = synth_qi_packet(SeqId::D, false, &[0x03, 0x00, 0x03], false);
        let cfg = DdsssConfig {
            sequence: DdsssSequence::Auto,
            extension: DdsssExtension::Auto,
            fop_hz: Some(FOP),
        };
        let ra = decode_ddsss(&a, &cfg);
        let rd = decode_ddsss(&d, &cfg);
        assert!(ra.info.contains("SEQA"), "info={}", ra.info);
        assert_eq!(ra.frames[0].bytes[0], 0x03);
        assert!(rd.info.contains("SEQD"), "info={}", rd.info);
        assert_eq!(rd.frames[0].bytes[0], 0x03);
    }

    #[test]
    fn decodes_multiple_qi_packets() {
        let packets = demo_packets();
        let trace = synthesize_ddsss(&DdsssSynthRequest {
            packets: packets.clone(),
            ..Default::default()
        })
        .expect("synth");
        let r = decode_ddsss(
            &trace,
            &DdsssConfig {
                sequence: DdsssSequence::SeqA,
                extension: DdsssExtension::Off,
                fop_hz: Some(FOP),
            },
        );
        assert!(r.error.is_none(), "{r:?}");
        let got: Vec<u8> = r.frames.iter().map(|f| f.bytes[0]).collect();
        let want: Vec<u8> = packets.iter().map(|p| p[0]).collect();
        assert_eq!(got, want, "info={} frames={:?}", r.info, r.frames);
        assert!(r.frames.iter().all(|f| f.t_end > f.t_start));
        assert_eq!(r.byte_spans.len(), packets.iter().map(|p| p.len()).sum::<usize>());
        assert!(!r.chips.is_empty());
    }

    #[test]
    fn decodes_ce_at_85khz() {
        let fop = 85_000.0;
        let trace = synthesize_ddsss(&DdsssSynthRequest {
            fop_hz: fop,
            packets: vec![qi_ask_frame(0x03, &[0x00])],
            ..Default::default()
        })
        .expect("synth");
        let r = decode_ddsss(
            &trace,
            &DdsssConfig {
                sequence: DdsssSequence::SeqA,
                extension: DdsssExtension::Off,
                fop_hz: Some(fop),
            },
        );
        assert_eq!(
            r.frames.first().map(|f| f.bytes.as_slice()),
            Some([0x03, 0x00, 0x03].as_slice()),
            "info={} err={:?}",
            r.info,
            r.error
        );
    }

    #[test]
    fn decodes_shallow_modulation() {
        let trace = synthesize_ddsss(&DdsssSynthRequest {
            amp_lo: 0.985,
            amp_hi: 1.0,
            packets: vec![qi_ask_frame(0x03, &[0x00])],
            ..Default::default()
        })
        .expect("synth");
        let r = decode_ddsss(
            &trace,
            &DdsssConfig {
                sequence: DdsssSequence::SeqA,
                extension: DdsssExtension::Off,
                fop_hz: Some(FOP),
            },
        );
        assert_eq!(
            r.frames.first().map(|f| f.bytes.as_slice()),
            Some([0x03, 0x00, 0x03].as_slice()),
            "info={} err={:?}",
            r.info,
            r.error
        );
    }

    fn demo_packets() -> Vec<Vec<u8>> {
        vec![
            qi_ask_frame(0x01, &[0x80]),
            qi_ask_frame(0x03, &[0x00]),
            qi_ask_frame(0x04, &[0x40]),
            qi_ask_frame(0x05, &[0x64]),
            qi_ask_frame(0x71, &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]),
        ]
    }

    /// Identification → config → transfer → end, covering common ASK headers.
    fn rich_demo_packets() -> Vec<Vec<u8>> {
        vec![
            qi_ask_frame(0x01, &[0x80]),
            qi_ask_frame(0x71, &[0x13, 0x00, 0x5A, 0x80, 0x12, 0x34, 0x56]),
            qi_ask_frame(0x81, &[0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44]),
            qi_ask_frame(0x51, &[0x0A, 0x00, 0x00, 0x20, 0x00]),
            qi_ask_frame(0x06, &[0x05]),
            qi_ask_frame(0x07, &[0x31]),
            qi_ask_frame(0x20, &[0x00, 0x00]),
            qi_ask_frame(0x03, &[0x00]),
            qi_ask_frame(0x04, &[0x40]),
            qi_ask_frame(0x31, &[0x00, 0x01, 0xF4]),
            qi_ask_frame(0x05, &[0x64]),
            qi_ask_frame(0x22, &[0x00, 0x10]),
            qi_ask_frame(0x15, &[0x00]),
            qi_ask_frame(0x09, &[0x00]),
            qi_ask_frame(0x02, &[0x01]),
        ]
    }

    fn seqa_cfg() -> DdsssConfig {
        DdsssConfig {
            sequence: DdsssSequence::SeqA,
            extension: DdsssExtension::Off,
            fop_hz: Some(FOP),
        }
    }

    fn packet_amps(seq: SeqId, extension: bool, packet: &[u8], invert: bool) -> Vec<f64> {
        let pattern = seq.bits(extension);
        let mut bits = vec![true; 14];
        for &b in packet {
            bits.extend(qi_byte_bits(b));
        }
        bits.extend(std::iter::repeat(true).take(12));
        chips_to_amps(&data_bits_to_chips(&bits, &pattern), invert, A_LO, A_HI)
    }

    fn synth_qi_packet(seq: SeqId, extension: bool, packet: &[u8], invert: bool) -> WaveformTrace {
        let sequence = match seq {
            SeqId::A => DdsssSequence::SeqA,
            SeqId::B => DdsssSequence::SeqB,
            SeqId::C => DdsssSequence::SeqC,
            SeqId::D => DdsssSequence::SeqD,
        };
        synthesize_ddsss(&DdsssSynthRequest {
            sequence,
            extension,
            invert,
            packets: vec![packet.to_vec()],
            ..Default::default()
        })
        .expect("synth")
    }

    fn amps_to_cycle_chips(amps: &[f64]) -> Vec<Option<bool>> {
        let trace = super::amps_to_trace(amps, FOP, SPC, "VCTX");
        let (env, _) = cycle_envelope(&trace, FOP);
        differential_chips(&env, 0.15)
    }

    fn cycle_phase_corr(cycle_chips: &[Option<bool>], start: usize, phase: usize, seq: &[bool]) -> u32 {
        let mut matches = 0u32;
        for (k, want) in seq.iter().enumerate() {
            let idx = start + phase + 2 * k;
            if let Some(Some(bit)) = cycle_chips.get(idx) {
                if *bit == *want {
                    matches += 1;
                }
            }
        }
        matches
    }

    #[test]
    fn export_example_ddsss_vctx() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples");
        std::fs::create_dir_all(&dir).expect("examples dir");
        let trace = synthesize_ddsss(&DdsssSynthRequest {
            samples_per_cycle: 8,
            packets: demo_packets(),
            ..Default::default()
        })
        .expect("synth");
        let isf = dir.join("ddsss_vctx.isf");
        crate::waveform_file::export_waveform_isf(&isf, &trace).expect("isf");
        assert!(isf.is_file());
        let loaded = crate::waveform_file::load_waveform_file_all(&isf).expect("reload");
        assert!(!loaded.is_empty());
        let r = decode_ddsss(
            &loaded[0],
            &DdsssConfig {
                sequence: DdsssSequence::SeqA,
                extension: DdsssExtension::Off,
                fop_hz: Some(FOP),
            },
        );
        assert_eq!(r.frames.len(), 5, "info={} err={:?}", r.info, r.error);
    }

    #[test]
    fn keeps_parity_error_frame_with_good_checksum() {
        let pkt = qi_ask_frame(0x03, &[0x00]);
        let trace = synthesize_ddsss(&DdsssSynthRequest {
            packets: vec![pkt.clone()],
            parity_errors: vec![0],
            ..Default::default()
        })
        .expect("synth");
        let r = decode_ddsss(&trace, &seqa_cfg());
        assert_eq!(r.frames.len(), 1, "info={} err={:?}", r.info, r.error);
        assert_eq!(r.frames[0].bytes, pkt);
        assert_eq!(r.frames[0].error, BusFrameError::Parity);
        assert_eq!(r.frames[0].plot_label(), "CE P!");
        assert!(r.byte_spans.iter().any(|b| b.error == BusByteError::Parity));
        assert!(r.bits.iter().any(|b| b.error && b.label() == "P!"));
    }

    #[test]
    fn chip_errors_are_marked_while_bits_still_decode() {
        let pkt = qi_ask_frame(0x03, &[0x00]);
        let trace = synthesize_ddsss(&DdsssSynthRequest {
            packets: vec![pkt.clone()],
            chip_errors: 8,
            ..Default::default()
        })
        .expect("synth");
        let r = decode_ddsss(&trace, &seqa_cfg());
        assert_eq!(r.frames.len(), 1, "info={} err={:?}", r.info, r.error);
        assert_eq!(r.frames[0].bytes, pkt);
        assert_eq!(r.frames[0].error, BusFrameError::None);
        let nerr = r.chips.iter().filter(|c| c.error).count();
        assert!(nerr >= 4, "chip_err={nerr} info={}", r.info);
        assert!(r.info.contains("chip_err="), "{}", r.info);
    }

    #[test]
    fn keeps_checksum_error_frame_and_following_packet() {
        let trace = synthesize_ddsss(&DdsssSynthRequest {
            packets: vec![qi_ask_frame(0x03, &[0x00]), qi_ask_frame(0x05, &[0x64])],
            checksum_errors: vec![0],
            ..Default::default()
        })
        .expect("synth");
        let r = decode_ddsss(&trace, &seqa_cfg());
        assert!(r.error.is_none(), "{r:?}");
        assert_eq!(
            r.frames
                .iter()
                .map(|f| f.plot_label())
                .collect::<Vec<_>>(),
            vec!["CE!".to_string(), "CHS".to_string()],
            "info={} frames={:?}",
            r.info,
            r.frames
        );
        assert_eq!(r.frames[0].error, BusFrameError::Checksum);
        assert_eq!(r.frames[0].bytes[2], 0x02);
        assert!(r.info.contains("cs_err=1"), "info={}", r.info);
    }

    #[test]
    fn decodes_rich_packets_with_chip_and_checksum_errors() {
        let packets = rich_demo_packets();
        let chs_i = 10;
        let trace = synthesize_ddsss(&DdsssSynthRequest {
            packets: packets.clone(),
            chip_errors: 24,
            checksum_errors: vec![chs_i],
            parity_errors: vec![7],
            ..Default::default()
        })
        .expect("synth");
        let r = decode_ddsss(&trace, &seqa_cfg());
        assert!(r.error.is_none(), "{r:?}");
        let names: Vec<String> = r.frames.iter().map(|f| f.plot_label()).collect();
        assert_eq!(names.len(), packets.len(), "info={} names={names:?}", r.info);
        assert_eq!(names[0], "SS");
        assert_eq!(names[1], "ID");
        assert_eq!(names[2], "XID");
        assert_eq!(names[3], "CFG");
        assert_eq!(names[7], "CE P!");
        assert_eq!(r.frames[7].error, BusFrameError::Parity);
        assert_eq!(names[chs_i], "CHS!");
        assert_eq!(r.frames[chs_i].error, BusFrameError::Checksum);
        assert_eq!(names.last().unwrap(), "EPT");
        assert!(names.iter().any(|n| n == "RP"));
        assert!(names.iter().any(|n| n == "GRQ"));
        assert!(names.iter().any(|n| n == "NEGO"));
        assert!(r.info.contains("cs_err=1"), "info={}", r.info);
        assert!(r.info.contains("P_err=1"), "info={}", r.info);
        let nchip_err = r.chips.iter().filter(|c| c.error).count();
        assert!(
            nchip_err >= 8,
            "expected chip mismatches from synth, got {nchip_err} info={}",
            r.info
        );
        assert!(r.info.contains("chip_err="), "info={}", r.info);
    }

    #[test]
    fn export_example_ddsss_vctx_errors() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples");
        std::fs::create_dir_all(&dir).expect("examples dir");
        let packets = rich_demo_packets();
        let trace = synthesize_ddsss(&DdsssSynthRequest {
            samples_per_cycle: 8,
            packets: packets.clone(),
            chip_errors: 24,
            checksum_errors: vec![10],
            parity_errors: vec![7],
            ..Default::default()
        })
        .expect("synth");
        let isf = dir.join("ddsss_vctx_errors.isf");
        crate::waveform_file::export_waveform_isf(&isf, &trace).expect("isf");
        assert!(isf.is_file());
        let loaded = crate::waveform_file::load_waveform_file_all(&isf).expect("reload");
        let r = decode_ddsss(&loaded[0], &seqa_cfg());
        assert_eq!(
            r.frames.len(),
            packets.len(),
            "info={} err={:?} names={:?}",
            r.info,
            r.error,
            r.frames.iter().map(|f| f.plot_label()).collect::<Vec<_>>()
        );
        assert_eq!(r.frames[7].plot_label(), "CE P!");
        assert_eq!(r.frames[10].plot_label(), "CHS!");
        assert!(
            r.chips.iter().any(|c| c.error),
            "exported ISF should keep chip mismatches info={}",
            r.info
        );
    }
}
