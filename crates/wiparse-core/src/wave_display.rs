//! Display-only waveform decimation and viewport LOD.
//!
//! Measurement, export, and cursors always use full [`WaveformTrace`] data elsewhere.
//! This module implements oscilloscope-style column envelopes and zoom-aware level-of-detail.

use std::sync::Arc;

use crate::waveform_file::WaveformMeasurements;

/// One screen column: time span `[x0, x1]` with min/max amplitude in that bucket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeEnvelopeColumn {
    pub x0: f64,
    pub x1: f64,
    pub y_min: f64,
    pub y_max: f64,
}

impl ScopeEnvelopeColumn {
    /// True when the bucket has no vertical extent (flat plateau / DC within column).
    #[inline]
    pub fn is_flat(&self) -> bool {
        let scale = self.y_min.abs().max(self.y_max.abs()).max(1.0);
        (self.y_max - self.y_min).abs() <= f64::EPSILON * scale * 8.0
    }
}

/// Overview envelope width (~2× 1080p); sufficient for full-trace pan/zoom overview.
pub const OVERVIEW_COLUMNS_DEFAULT: usize = 4096;
/// Viewport decimation cap (~2× 4K monitor width).
pub const VIEWPORT_COLUMNS_MAX: usize = 8192;
pub const VIEWPORT_COLUMNS_MIN: usize = 512;
/// When visible samples ≤ `cols × MULT`, draw time-ordered samples (digital edges).
const UNIFORM_LOD_MULT: usize = 4;
const UNIFORM_SAMPLE_CAP: usize = 131_072;

#[derive(Clone)]
pub enum WaveViewportSeries {
    Envelope(Arc<Vec<ScopeEnvelopeColumn>>),
    /// Time-ordered polyline when zoomed in enough to resolve transitions.
    Uniform(Arc<Vec<[f64; 2]>>),
}

impl WaveViewportSeries {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Envelope(c) => c.is_empty(),
            Self::Uniform(p) => p.len() < 2,
        }
    }
}

#[inline]
pub fn overview_column_count() -> usize {
    OVERVIEW_COLUMNS_DEFAULT
}

/// ~2 display pixels per column (scope-style density).
#[inline]
pub fn viewport_column_count(plot_width_px: f32) -> usize {
    ((plot_width_px * 2.0).ceil() as usize).clamp(VIEWPORT_COLUMNS_MIN, VIEWPORT_COLUMNS_MAX)
}

/// Build global overview envelope for a full trace.
pub fn build_overview_envelope(x: &[f64], y: &[f64]) -> Arc<Vec<ScopeEnvelopeColumn>> {
    build_load_snapshot(x, y).0
}

/// Quick stats + overview envelope in a single O(N) pass (tier-1 file load).
#[derive(Debug, Clone, Copy)]
pub struct TraceLoadSnapshot {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub rms: f64,
    pub dt: f64,
}

impl TraceLoadSnapshot {
    pub fn to_quick_measures(&self) -> WaveformMeasurements {
        WaveformMeasurements {
            count: self.count,
            dt: self.dt,
            min: self.min,
            max: self.max,
            pp: self.max - self.min,
            mean: self.mean,
            rms: self.rms,
            freq_hz: None,
            period: None,
        }
    }
}

/// One-pass overview envelope + autoscale extent + basic measurements (no frequency).
pub fn build_load_snapshot(x: &[f64], y: &[f64]) -> (Arc<Vec<ScopeEnvelopeColumn>>, TraceLoadSnapshot) {
    let n = x.len().min(y.len());
    if n == 0 {
        return (
            Arc::new(Vec::new()),
            TraceLoadSnapshot {
                x_min: 0.0,
                x_max: 1.0,
                y_min: 0.0,
                y_max: 1.0,
                count: 0,
                min: 0.0,
                max: 0.0,
                mean: 0.0,
                rms: 0.0,
                dt: 0.0,
            },
        );
    }
    let max_columns = overview_column_count();
    let (columns, stats) = if n >= 2 && x_is_monotonic(x, n) {
        decimate_envelope_time_buckets_with_stats(x, y, n, max_columns)
    } else {
        decimate_envelope_index_buckets_with_stats(x, y, n, max_columns)
    };
    let dt = if n >= 2 {
        (x[n - 1] - x[0]) / (n as f64 - 1.0)
    } else {
        0.0
    };
    let (mean, rms) = stats.mean_rms();
    let snapshot = TraceLoadSnapshot {
        x_min: stats.x_min,
        x_max: stats.x_max,
        y_min: stats.y_min,
        y_max: stats.y_max,
        count: stats.count,
        min: stats.y_min,
        max: stats.y_max,
        mean,
        rms,
        dt,
    };
    (Arc::new(columns), snapshot)
}

#[derive(Default)]
struct LoadPassStats {
    count: usize,
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    sum: f64,
    sum_sq: f64,
}

impl LoadPassStats {
    fn new() -> Self {
        Self {
            x_min: f64::INFINITY,
            x_max: f64::NEG_INFINITY,
            y_min: f64::INFINITY,
            y_max: f64::NEG_INFINITY,
            ..Default::default()
        }
    }

    fn add_sample(&mut self, x: f64, y: f64) {
        if x.is_finite() {
            self.x_min = self.x_min.min(x);
            self.x_max = self.x_max.max(x);
        }
        if y.is_finite() {
            self.count += 1;
            self.y_min = self.y_min.min(y);
            self.y_max = self.y_max.max(y);
            self.sum += y;
            self.sum_sq += y * y;
        }
    }

    fn mean_rms(&self) -> (f64, f64) {
        if self.count == 0 {
            return (0.0, 0.0);
        }
        let mean = self.sum / self.count as f64;
        let rms = (self.sum_sq / self.count as f64).sqrt();
        (mean, rms)
    }

    fn finalize_bounds(&mut self) {
        if !self.x_min.is_finite() {
            self.x_min = 0.0;
            self.x_max = 1.0;
        }
        if !self.y_min.is_finite() {
            self.y_min = 0.0;
            self.y_max = 1.0;
        }
    }
}

fn decimate_envelope_index_buckets_with_stats(
    x: &[f64],
    y: &[f64],
    n: usize,
    max_columns: usize,
) -> (Vec<ScopeEnvelopeColumn>, LoadPassStats) {
    let mut out = Vec::with_capacity(max_columns);
    let mut stats = LoadPassStats::new();
    for b in 0..max_columns {
        let start = b * n / max_columns;
        let end = ((b + 1) * n / max_columns).max(start + 1).min(n);
        if let Some(col) = envelope_column_range_with_stats(x, y, start, end, &mut stats) {
            out.push(col);
        }
    }
    stats.finalize_bounds();
    (out, stats)
}

fn decimate_envelope_time_buckets_with_stats(
    x: &[f64],
    y: &[f64],
    n: usize,
    max_columns: usize,
) -> (Vec<ScopeEnvelopeColumn>, LoadPassStats) {
    let mut x0 = x[0];
    let mut x1 = x[n - 1];
    if !x0.is_finite() || !x1.is_finite() {
        return decimate_envelope_index_buckets_with_stats(x, y, n, max_columns);
    }
    if x1 < x0 {
        std::mem::swap(&mut x0, &mut x1);
    }
    let span = (x1 - x0).abs().max(f64::EPSILON);
    let mut out = Vec::with_capacity(max_columns);
    let mut stats = LoadPassStats::new();
    let mut idx = 0usize;
    for b in 0..max_columns {
        let t0 = x0 + span * (b as f64) / max_columns as f64;
        let t1 = if b + 1 == max_columns {
            x1 + f64::EPSILON
        } else {
            x0 + span * ((b + 1) as f64) / max_columns as f64
        };
        while idx < n && x[idx] < t0 {
            idx += 1;
        }
        let start = idx;
        while idx < n && x[idx] <= t1 {
            idx += 1;
        }
        let end = idx.max(start + 1).min(n);
        if let Some(col) = envelope_column_range_with_stats(x, y, start, end, &mut stats) {
            out.push(col);
        }
    }
    stats.finalize_bounds();
    (out, stats)
}

fn envelope_column_range_with_stats(
    x: &[f64],
    y: &[f64],
    start: usize,
    end: usize,
    stats: &mut LoadPassStats,
) -> Option<ScopeEnvelopeColumn> {
    if start >= end {
        return None;
    }
    let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut any = false;
    for i in start..end {
        let xv = x[i];
        let yv = y[i];
        stats.add_sample(xv, yv);
        if yv.is_finite() {
            any = true;
            ymin = ymin.min(yv);
            ymax = ymax.max(yv);
        }
    }
    if !any {
        return None;
    }
    let mut col_x0 = x[start];
    let mut col_x1 = x[end - 1];
    if col_x1 < col_x0 {
        std::mem::swap(&mut col_x0, &mut col_x1);
    }
    if (col_x1 - col_x0).abs() < f64::EPSILON {
        col_x1 = col_x0;
    }
    Some(ScopeEnvelopeColumn {
        x0: col_x0,
        x1: col_x1,
        y_min: ymin,
        y_max: ymax,
    })
}

/// Scope-style min/max envelope with explicit column time span (professional display).
pub fn decimate_envelope_columns(
    x: &[f64],
    y: &[f64],
    max_columns: usize,
) -> Vec<ScopeEnvelopeColumn> {
    let n = x.len().min(y.len());
    if n == 0 {
        return Vec::new();
    }
    let max_columns = max_columns.max(1);
    if n >= 2 && x_is_monotonic(x, n) {
        return decimate_envelope_time_buckets(x, y, n, max_columns);
    }
    decimate_envelope_index_buckets(x, y, n, max_columns)
}

#[inline]
fn x_is_monotonic(x: &[f64], n: usize) -> bool {
    x[..n].windows(2).all(|w| w[1] >= w[0] || (w[1] - w[0]).abs() < 1e-18)
}

fn decimate_envelope_index_buckets(
    x: &[f64],
    y: &[f64],
    n: usize,
    max_columns: usize,
) -> Vec<ScopeEnvelopeColumn> {
    let mut out = Vec::with_capacity(max_columns);
    for b in 0..max_columns {
        let start = b * n / max_columns;
        let end = ((b + 1) * n / max_columns).max(start + 1).min(n);
        if let Some(col) = envelope_column_range(x, y, start, end) {
            out.push(col);
        }
    }
    out
}

fn decimate_envelope_time_buckets(
    x: &[f64],
    y: &[f64],
    n: usize,
    max_columns: usize,
) -> Vec<ScopeEnvelopeColumn> {
    let mut x0 = x[0];
    let mut x1 = x[n - 1];
    if !x0.is_finite() || !x1.is_finite() {
        return decimate_envelope_index_buckets(x, y, n, max_columns);
    }
    if x1 < x0 {
        std::mem::swap(&mut x0, &mut x1);
    }
    let span = (x1 - x0).abs().max(f64::EPSILON);
    let mut out = Vec::with_capacity(max_columns);
    let mut idx = 0usize;
    for b in 0..max_columns {
        let t0 = x0 + span * (b as f64) / max_columns as f64;
        let t1 = if b + 1 == max_columns {
            x1 + f64::EPSILON
        } else {
            x0 + span * ((b + 1) as f64) / max_columns as f64
        };
        while idx < n && x[idx] < t0 {
            idx += 1;
        }
        let start = idx;
        while idx < n && x[idx] <= t1 {
            idx += 1;
        }
        let end = idx.max(start + 1).min(n);
        if let Some(col) = envelope_column_range(x, y, start, end) {
            out.push(col);
        }
    }
    out
}

fn envelope_column_range(
    x: &[f64],
    y: &[f64],
    start: usize,
    end: usize,
) -> Option<ScopeEnvelopeColumn> {
    if start >= end {
        return None;
    }
    let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut any = false;
    for i in start..end {
        let v = y[i];
        if v.is_finite() {
            any = true;
            ymin = ymin.min(v);
            ymax = ymax.max(v);
        }
    }
    if !any {
        return None;
    }
    let mut x0 = x[start];
    let mut x1 = x[end - 1];
    if x1 < x0 {
        std::mem::swap(&mut x0, &mut x1);
    }
    if (x1 - x0).abs() < f64::EPSILON {
        x1 = x0;
    }
    Some(ScopeEnvelopeColumn {
        x0,
        x1,
        y_min: ymin,
        y_max: ymax,
    })
}

/// Union bounds of envelope columns `(xmin, xmax, ymin, ymax)`.
pub fn envelope_bounds(columns: &[ScopeEnvelopeColumn]) -> (f64, f64, f64, f64) {
    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for c in columns {
        xmin = xmin.min(c.x0).min(c.x1);
        xmax = xmax.max(c.x0).max(c.x1);
        ymin = ymin.min(c.y_min);
        ymax = ymax.max(c.y_max);
    }
    if !xmin.is_finite() {
        return (0.0, 1.0, 0.0, 1.0);
    }
    (xmin, xmax, ymin, ymax)
}

/// Viewport LOD: envelope columns, or uniform samples when zoomed to sample resolution.
pub fn build_viewport_series(
    x: &[f64],
    y: &[f64],
    x_view0: f64,
    x_view1: f64,
    plot_width_px: f32,
) -> WaveViewportSeries {
    let n = x.len().min(y.len());
    if n == 0 {
        return WaveViewportSeries::Envelope(Arc::new(Vec::new()));
    }
    let lo = x_view0.min(x_view1);
    let hi = x_view0.max(x_view1);
    let pad = (hi - lo).abs() * 0.02;
    let start = x[..n]
        .partition_point(|&v| v < lo - pad)
        .saturating_sub(1);
    let end = x[..n]
        .partition_point(|&v| v <= hi + pad)
        .min(n)
        .max(start + 1);
    let vis = end.saturating_sub(start);
    let cols = viewport_column_count(plot_width_px);
    let uniform_threshold = cols.saturating_mul(UNIFORM_LOD_MULT).max(512);

    if vis <= uniform_threshold && vis >= 2 {
        let mut pts = Vec::with_capacity(vis.min(UNIFORM_SAMPLE_CAP));
        for i in start..end {
            if pts.len() >= UNIFORM_SAMPLE_CAP {
                break;
            }
            let xv = x[i];
            let yv = y[i];
            if xv.is_finite() && yv.is_finite() {
                pts.push([xv, yv]);
            }
        }
        if pts.len() >= 2 {
            return WaveViewportSeries::Uniform(Arc::new(pts));
        }
    }

    let xs = &x[start..end];
    let ys = &y[start..end];
    WaveViewportSeries::Envelope(Arc::new(decimate_envelope_columns(xs, ys, cols)))
}

/// Quantize pan/zoom X window so viewport cache stays stable while dragging.
pub fn quantize_view_cache_key(x0: f64, x1: f64, xmin: f64, xmax: f64, bins: usize) -> u64 {
    let (lo, hi) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    let span = (xmax - xmin).abs().max(1e-30);
    let bins = bins.max(64) as f64;
    let b0 = (((lo - xmin) / span) * bins).floor().clamp(0.0, bins) as u32;
    let b1 = (((hi - xmin) / span) * bins).ceil().clamp(0.0, bins) as u32;
    (bins as u64) << 48 | (b0 as u64) << 24 | (b1 as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_plateau_columns_are_flat() {
        let x: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let y = vec![5.0; 100];
        let cols = decimate_envelope_columns(&x, &y, 10);
        assert!(!cols.is_empty());
        assert!(cols.iter().all(|c| c.is_flat()));
        assert!(cols.iter().all(|c| (c.y_min - 5.0).abs() < 1e-9));
    }

    #[test]
    fn square_wave_columns_have_spans() {
        let x: Vec<f64> = (0..1000).map(|i| i as f64 * 1e-6).collect();
        let y: Vec<f64> = x.iter().map(|t| if (*t * 1e6 as f64) as i32 % 200 < 100 { 1.0 } else { 0.0 }).collect();
        let cols = decimate_envelope_columns(&x, &y, 50);
        assert!(cols.len() >= 10);
        let has_edge = cols.iter().any(|c| !c.is_flat());
        let has_flat = cols.iter().any(|c| c.is_flat());
        assert!(has_edge && has_flat);
    }

    #[test]
    fn viewport_uniform_when_few_samples() {
        let x: Vec<f64> = (0..64).map(|i| i as f64).collect();
        let y: Vec<f64> = x.iter().map(|v| if *v as i32 % 2 == 0 { 0.0 } else { 1.0 }).collect();
        match build_viewport_series(&x, &y, 0.0, 63.0, 800.0) {
            WaveViewportSeries::Uniform(p) => assert!(p.len() >= 32),
            WaveViewportSeries::Envelope(_) => panic!("expected uniform LOD for small window"),
        }
    }
}
