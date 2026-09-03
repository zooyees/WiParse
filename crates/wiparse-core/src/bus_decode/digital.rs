//! Analog → digital conversion using MCU CMOS VIL / VIH (Schmitt).
//!
//! Thresholds are in **volts relative to each channel's native 0 (GND)**, using
//! that channel's `y_unit` scale (V / mV / µV). Plot stacking offset and the
//! on-screen Y ruler are display-only and must not be used.
//!
//! Typical MCU GPIO (JEDEC CMOS): VIL max = 0.3 VDD, VIH min = 0.7 VDD.
//! VDD is snapped to 1.8 / 3.3 / 5 V from the high side vs GND, defaulting to
//! 3.3 V so millivolt analog jitter around ground cannot become a logic swing.

use crate::instrument::WaveformTrace;

/// CMOS VIL as a fraction of VDD (from channel GND = 0 V).
const CMOS_VIL_FRAC: f64 = 0.3;
/// CMOS VIH as a fraction of VDD (from channel GND = 0 V).
const CMOS_VIH_FRAC: f64 = 0.7;
/// Half of the CMOS undefined window (VIH − VIL = 0.4 VDD).
const CMOS_UNDEF_HALF: f64 = 0.2;
/// Default MCU I/O rail when the trace never reaches a valid high.
const DEFAULT_VDD: f64 = 3.3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Rising,
    Falling,
}

#[derive(Debug, Clone, Copy)]
pub struct DigitalEdge {
    pub time: f64,
    pub kind: EdgeKind,
    pub level_after: bool,
}

/// Schmitt thresholds inferred from a trace (or centered on a user threshold).
#[derive(Debug, Clone, Copy)]
pub struct LogicLevels {
    pub vil: f64,
    pub vih: f64,
}

impl LogicLevels {
    /// VIL / VIH in volts from this channel's GND (native 0), not from plot Y.
    ///
    /// If `threshold` is set (volts), the CMOS undefined window is centered on it.
    pub fn from_trace(trace: &WaveformTrace, threshold: Option<f64>) -> Self {
        let vdd = infer_vdd(trace);
        let (mut vil, mut vih) = match threshold.filter(|t| t.is_finite()) {
            Some(thr) => {
                let thr = sample_volts(thr, &trace.y_unit);
                (thr - CMOS_UNDEF_HALF * vdd, thr + CMOS_UNDEF_HALF * vdd)
            }
            None => (CMOS_VIL_FRAC * vdd, CMOS_VIH_FRAC * vdd),
        };
        if vih - vil < 1e-9 {
            let mid = 0.5 * (vil + vih);
            vil = mid - 1e-3;
            vih = mid + 1e-3;
        }
        Self { vil, vih }
    }

    pub fn midpoint(self) -> f64 {
        0.5 * (self.vil + self.vih)
    }

    fn initial(self, volts: f64) -> bool {
        // Until the pin is a valid CMOS 0 vs GND, treat it as 1 (idle-high).
        volts > self.vil
    }

    fn classify(self, volts: f64, prev: bool) -> bool {
        if volts >= self.vih {
            true
        } else if volts <= self.vil {
            false
        } else {
            prev
        }
    }
}

/// Digitized trace: CMOS Schmitt levels aligned with analog samples.
pub struct LogicWave<'a> {
    pub trace: &'a WaveformTrace,
    pub levels: LogicLevels,
    bits: Vec<bool>,
}

impl<'a> LogicWave<'a> {
    pub fn new(trace: &'a WaveformTrace, threshold: Option<f64>) -> Self {
        let levels = LogicLevels::from_trace(trace, threshold);
        Self {
            bits: digitize(trace, levels),
            trace,
            levels,
        }
    }

    pub fn edges(&self) -> Vec<DigitalEdge> {
        edges_from_bits(self.trace, &self.bits)
    }

    /// Logic level at time `t` (last sample with x ≤ t).
    pub fn at(&self, t: f64) -> Option<bool> {
        digital_at_or_before(&self.trace.x, &self.bits, t)
    }

    /// Logic level of the last sample strictly before `t` (setup time).
    pub fn before(&self, t: f64) -> Option<bool> {
        digital_before(&self.trace.x, &self.bits, t)
    }

    /// First sample in volts vs this channel's GND.
    pub fn first_volts(&self) -> Option<f64> {
        self.trace
            .y
            .first()
            .copied()
            .map(|y| sample_volts(y, &self.trace.y_unit))
    }
}

/// Mid-point of CMOS VIL/VIH (kept for UI / fallback).
pub fn default_threshold(trace: &WaveformTrace) -> f64 {
    LogicLevels::from_trace(trace, None).midpoint()
}

fn digitize(trace: &WaveformTrace, levels: LogicLevels) -> Vec<bool> {
    let n = trace.x.len().min(trace.y.len());
    let mut bits = Vec::with_capacity(n);
    let mut prev = false;
    let mut started = false;
    for i in 0..n {
        let y = trace.y[i];
        if !y.is_finite() {
            bits.push(if started { prev } else { false });
            continue;
        }
        let volts = sample_volts(y, &trace.y_unit);
        if !started {
            prev = levels.initial(volts);
            started = true;
        } else {
            prev = levels.classify(volts, prev);
        }
        bits.push(prev);
    }
    bits
}

/// Native sample → volts using the channel's unit scale (0 = that channel's GND).
pub fn sample_volts(y: f64, y_unit: &str) -> f64 {
    let u = y_unit.trim();
    if u.eq_ignore_ascii_case("mV") {
        y * 1e-3
    } else if u.eq_ignore_ascii_case("uV") || u == "μV" || u == "µV" {
        y * 1e-6
    } else if u.eq_ignore_ascii_case("kV") {
        y * 1e3
    } else {
        y
    }
}

fn infer_vdd(trace: &WaveformTrace) -> f64 {
    let (_lo, hi) = robust_volt_extents(trace);
    let high = if hi.is_finite() { hi.max(0.0) } else { 0.0 };
    if high >= 4.0 {
        5.0
    } else if high >= 2.4 {
        3.3
    } else if high >= 1.5 {
        1.8
    } else {
        DEFAULT_VDD
    }
}

fn robust_volt_extents(trace: &WaveformTrace) -> (f64, f64) {
    let n = trace.y.len();
    if n == 0 {
        return (f64::NAN, f64::NAN);
    }
    let step = (n / 4096).max(1);
    let mut samples: Vec<f64> = trace
        .y
        .iter()
        .step_by(step)
        .copied()
        .filter(|v| v.is_finite())
        .map(|y| sample_volts(y, &trace.y_unit))
        .collect();
    if samples.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let last = samples.len() - 1;
    let lo = samples[(samples.len() / 20).min(last)];
    let hi = samples[(samples.len() * 19 / 20).min(last)];
    (lo, hi)
}

fn edges_from_bits(trace: &WaveformTrace, bits: &[bool]) -> Vec<DigitalEdge> {
    let n = trace.x.len().min(bits.len());
    if n < 2 {
        return Vec::new();
    }
    let mut edges = Vec::new();
    for i in 1..n {
        if bits[i] == bits[i - 1] {
            continue;
        }
        edges.push(DigitalEdge {
            time: trace.x[i],
            kind: if bits[i] {
                EdgeKind::Rising
            } else {
                EdgeKind::Falling
            },
            level_after: bits[i],
        });
    }
    edges
}

/// Detect VIL/VIH crossings (Schmitt). `threshold` centers the CMOS window when set.
pub fn analog_to_edges(
    trace: &WaveformTrace,
    threshold: Option<f64>,
) -> Vec<DigitalEdge> {
    LogicWave::new(trace, threshold).edges()
}

fn digital_at_or_before(x: &[f64], bits: &[bool], t: f64) -> Option<bool> {
    let n = x.len().min(bits.len());
    if n == 0 {
        return None;
    }
    if t <= x[0] {
        return Some(bits[0]);
    }
    if t >= x[n - 1] {
        return Some(bits[n - 1]);
    }
    let idx = x[..n].partition_point(|&xi| xi < t);
    if idx < n && x[idx] <= t {
        Some(bits[idx])
    } else if idx == 0 {
        Some(bits[0])
    } else {
        Some(bits[idx - 1])
    }
}

fn digital_before(x: &[f64], bits: &[bool], t: f64) -> Option<bool> {
    let n = x.len().min(bits.len());
    if n == 0 {
        return None;
    }
    let idx = x[..n].partition_point(|&xi| xi < t);
    if idx == 0 {
        return Some(bits[0]);
    }
    Some(bits[idx - 1])
}

/// Pulse widths between consecutive edges.
pub fn edge_intervals(edges: &[DigitalEdge]) -> Vec<f64> {
    edges
        .windows(2)
        .map(|w| (w[1].time - w[0].time).abs())
        .filter(|dt| *dt > 0.0 && dt.is_finite())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(ys: &[f64]) -> WaveformTrace {
        WaveformTrace {
            channel: "CH".into(),
            x: (0..ys.len()).map(|i| i as f64 * 1e-6).collect::<Vec<_>>().into(),
            y: ys.to_vec().into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        }
    }

    #[test]
    fn cmos_levels_for_3v3() {
        let t = trace(&[0.0, 3.3, 0.0, 3.3, 0.05, 3.25]);
        let lv = LogicLevels::from_trace(&t, None);
        assert!((lv.vil - 0.3 * 3.3).abs() < 0.15, "{lv:?}");
        assert!((lv.vih - 0.7 * 3.3).abs() < 0.15, "{lv:?}");
    }

    #[test]
    fn midband_ringing_does_not_create_edges() {
        // Full CMOS swing so VIL/VIH track 0–3.3 V, then idle-high ringing
        // in the undefined band (1.2–2.0 V) must not add extra edges.
        let mut y = vec![0.0, 0.0, 3.3, 3.3];
        y.extend_from_slice(&[1.2, 1.9, 1.3, 2.0, 1.4, 1.8, 1.25, 1.95]);
        y.extend_from_slice(&[3.3, 3.3, 3.3]);
        let t = trace(&y);
        let edges = analog_to_edges(&t, None);
        assert_eq!(edges.len(), 1, "{edges:?}");
        assert_eq!(edges[0].kind, EdgeKind::Rising);
    }

    #[test]
    fn full_swing_produces_one_fall_and_one_rise() {
        let y = [3.3, 3.3, 0.0, 0.0, 3.3, 3.3];
        let t = trace(&y);
        let edges = analog_to_edges(&t, None);
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].kind, EdgeKind::Falling);
        assert_eq!(edges[1].kind, EdgeKind::Rising);
    }

    #[test]
    fn millivolt_jitter_around_ground_is_not_logic() {
        let y: Vec<f64> = (0..64)
            .map(|i| 0.04 * ((i % 6) as f64 - 2.5) / 2.5)
            .collect();
        let t = trace(&y);
        let lv = LogicLevels::from_trace(&t, None);
        assert!((lv.vil - 0.3 * 3.3).abs() < 0.05, "{lv:?}");
        assert!((lv.vih - 0.7 * 3.3).abs() < 0.05, "{lv:?}");
        let edges = analog_to_edges(&t, None);
        assert!(edges.is_empty(), "noise edges: {edges:?}");
    }

    #[test]
    fn millivolt_unit_scale_still_uses_volts_vs_gnd() {
        let t = WaveformTrace {
            channel: "CH".into(),
            x: (0..6).map(|i| i as f64 * 1e-6).collect::<Vec<_>>().into(),
            y: vec![0.0, 3300.0, 0.0, 3300.0, 50.0, 3250.0].into(),
            x_unit: "s".into(),
            y_unit: "mV".into(),
        };
        let lv = LogicLevels::from_trace(&t, None);
        assert!((lv.vil - 0.99).abs() < 0.05, "{lv:?}");
        let edges = analog_to_edges(&t, None);
        assert!(edges.len() >= 2);
    }
}
