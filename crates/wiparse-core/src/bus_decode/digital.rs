//! Analog → digital conversion and edge detection.

use crate::instrument::WaveformTrace;

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

/// Mid-point threshold from Y extrema (robust for scope digital probes).
pub fn default_threshold(trace: &WaveformTrace) -> f64 {
    let n = trace.y.len();
    if n == 0 {
        return 0.5;
    }
    let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
    for &v in &trace.y {
        if v.is_finite() {
            ymin = ymin.min(v);
            ymax = ymax.max(v);
        }
    }
    if ymin.is_finite() && ymax.is_finite() {
        0.5 * (ymin + ymax)
    } else {
        0.5
    }
}

fn level_at(y: f64, threshold: f64, hysteresis: f64, prev: bool) -> bool {
    if prev {
        y > threshold - hysteresis
    } else {
        y > threshold + hysteresis
    }
}

/// Detect threshold crossings with hysteresis (fraction of peak-to-peak).
pub fn analog_to_edges(
    trace: &WaveformTrace,
    threshold: f64,
    hysteresis_frac: f64,
) -> Vec<DigitalEdge> {
    let n = trace.x.len().min(trace.y.len());
    if n < 2 {
        return Vec::new();
    }
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for i in 0..n {
        let v = trace.y[i];
        if v.is_finite() {
            ymin = ymin.min(v);
            ymax = ymax.max(v);
        }
    }
    let span = (ymax - ymin).abs().max(1e-12);
    let hyst = span * hysteresis_frac.clamp(0.01, 0.25);

    let mut edges = Vec::new();
    let mut prev = level_at(trace.y[0], threshold, hyst, false);
    for i in 1..n {
        let y = trace.y[i];
        if !y.is_finite() {
            continue;
        }
        let cur = level_at(y, threshold, hyst, prev);
        if cur != prev {
            let kind = if cur {
                EdgeKind::Rising
            } else {
                EdgeKind::Falling
            };
            edges.push(DigitalEdge {
                time: trace.x[i],
                kind,
                level_after: cur,
            });
            prev = cur;
        }
    }
    edges
}

/// Sample logic level at time `t` via linear interpolation.
pub fn level_at_time(trace: &WaveformTrace, threshold: f64, t: f64) -> Option<bool> {
    let n = trace.x.len().min(trace.y.len());
    if n == 0 {
        return None;
    }
    if t <= trace.x[0] {
        return Some(trace.y[0] > threshold);
    }
    if t >= trace.x[n - 1] {
        return Some(trace.y[n - 1] > threshold);
    }
    let idx = trace.x[..n].partition_point(|&x| x < t);
    if idx == 0 || idx >= n {
        return None;
    }
    let x0 = trace.x[idx - 1];
    let x1 = trace.x[idx];
    let y0 = trace.y[idx - 1];
    let y1 = trace.y[idx];
    if (x1 - x0).abs() < f64::EPSILON {
        return Some(y0 > threshold);
    }
    let frac = (t - x0) / (x1 - x0);
    let y = y0 + frac * (y1 - y0);
    Some(y > threshold)
}

/// Pulse widths between consecutive edges.
pub fn edge_intervals(edges: &[DigitalEdge]) -> Vec<f64> {
    edges
        .windows(2)
        .map(|w| (w[1].time - w[0].time).abs())
        .filter(|dt| *dt > 0.0 && dt.is_finite())
        .collect()
}
