//! Display-only waveform decimation and viewport LOD.
//!
//! Measurement, export, and cursors always use full [`WaveformTrace`] data elsewhere.
//! This module implements oscilloscope-style MMFL (first/min/max/last) column envelopes
//! and samples-per-pixel LOD with hysteresis (Peak Detect vs Vector).

use std::sync::Arc;

use crate::waveform_file::WaveformMeasurements;

/// One display column: time span `[x0, x1]` with MMFL amplitude in that bucket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeEnvelopeColumn {
    pub x0: f64,
    pub x1: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub y_first: f64,
    pub y_last: f64,
    /// Sample time of `y_first` (not necessarily the bucket edge `x0`).
    pub x_first: f64,
    /// Sample time of `y_last`.
    pub x_last: f64,
    /// Sample time of `y_min`.
    pub x_min: f64,
    /// Sample time of `y_max`.
    pub x_max: f64,
    /// True when the min sample occurred at or before the max sample.
    pub min_before_max: bool,
    /// True when the samples reverse direction inside the bucket (Peak Detect).
    pub reversed: bool,
}

impl ScopeEnvelopeColumn {
    /// True when the bucket has no vertical extent (flat plateau / DC within column).
    #[inline]
    pub fn is_flat(&self) -> bool {
        let scale = self.y_min.abs().max(self.y_max.abs()).max(1.0);
        (self.y_max - self.y_min).abs() <= f64::EPSILON * scale * 8.0
    }

    /// True when min/max extend beyond the first–last segment (interior peak).
    #[inline]
    pub fn has_interior_extrema(&self) -> bool {
        let lo = self.y_first.min(self.y_last);
        let hi = self.y_first.max(self.y_last);
        let scale = self
            .y_min
            .abs()
            .max(self.y_max.abs())
            .max(hi.abs())
            .max(1.0);
        let eps = f64::EPSILON * scale * 32.0;
        self.y_min < lo - eps || self.y_max > hi + eps
    }

    /// Insert a min/max tick: interior peak, or a reversing (non-monotonic) bucket.
    #[inline]
    pub fn needs_peak_detect(&self) -> bool {
        self.reversed || self.has_interior_extrema()
    }

    /// Time-ordered first / interior min / interior max / last (2–4 unique samples).
    pub fn mmfl_points(&self) -> Vec<(f64, f64)> {
        let mut buf = [(0.0, 0.0); 4];
        let n = self.mmfl_points_n(&mut buf);
        buf[..n].to_vec()
    }

    /// Write MMFL vertices into `out` (max 4). Returns the count. No heap allocation.
    pub fn mmfl_points_n(&self, out: &mut [(f64, f64); 4]) -> usize {
        let mut n = 0usize;
        out[n] = (self.x_first, self.y_first);
        n += 1;
        if self.has_interior_extrema() {
            let lo = self.y_first.min(self.y_last);
            let hi = self.y_first.max(self.y_last);
            let scale = self
                .y_min
                .abs()
                .max(self.y_max.abs())
                .max(hi.abs())
                .max(1.0);
            let eps = f64::EPSILON * scale * 32.0;
            if self.y_min < lo - eps {
                out[n] = (self.x_min, self.y_min);
                n += 1;
            }
            if self.y_max > hi + eps {
                out[n] = (self.x_max, self.y_max);
                n += 1;
            }
        }
        out[n] = (self.x_last, self.y_last);
        n += 1;
        out[..n].sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut w = 0usize;
        for i in 0..n {
            if w > 0 && y_pair_near(out[w - 1], out[i]) {
                continue;
            }
            out[w] = out[i];
            w += 1;
        }
        w
    }
}

fn y_pair_near(a: (f64, f64), b: (f64, f64)) -> bool {
    let sx = a.0.abs().max(b.0.abs()).max(1.0);
    let sy = a.1.abs().max(b.1.abs()).max(1.0);
    (a.0 - b.0).abs() <= f64::EPSILON * sx * 32.0 && (a.1 - b.1).abs() <= f64::EPSILON * sy * 32.0
}

fn hold_column(x0: f64, x1: f64, y: f64) -> ScopeEnvelopeColumn {
    ScopeEnvelopeColumn {
        x0,
        x1,
        y_min: y,
        y_max: y,
        y_first: y,
        y_last: y,
        x_first: x0,
        x_last: x1,
        x_min: x0,
        x_max: x0,
        min_before_max: true,
        reversed: false,
    }
}

fn segment_column(x0: f64, y0: f64, x1: f64, y1: f64) -> ScopeEnvelopeColumn {
    let (x_min, x_max) = if y0 <= y1 { (x0, x1) } else { (x1, x0) };
    ScopeEnvelopeColumn {
        x0,
        x1,
        y_min: y0.min(y1),
        y_max: y0.max(y1),
        y_first: y0,
        y_last: y1,
        x_first: x0,
        x_last: x1,
        x_min,
        x_max,
        min_before_max: y0 <= y1,
        reversed: false,
    }
}

/// Overview envelope width (~2× 1080p); sufficient for full-trace pan/zoom overview.
pub const OVERVIEW_COLUMNS_DEFAULT: usize = 4096;
/// Viewport decimation cap (~2× 4K monitor width).
pub const VIEWPORT_COLUMNS_MAX: usize = 8192;
pub const VIEWPORT_COLUMNS_MIN: usize = 64;
/// Enter Vector (connected samples) below this samples-per-pixel.
const SPP_ENTER_VECTOR: f64 = 1.6;
/// Stay in Vector until spp exceeds this (hysteresis vs Peak Detect).
const SPP_LEAVE_VECTOR: f64 = 2.4;
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

    pub fn is_uniform(&self) -> bool {
        matches!(self, Self::Uniform(_))
    }
}

#[inline]
pub fn overview_column_count() -> usize {
    OVERVIEW_COLUMNS_DEFAULT
}

/// One display column per pixel (Tek/Keysight raster density).
#[inline]
pub fn viewport_column_count(plot_width_px: f32) -> usize {
    (plot_width_px.round() as usize).clamp(VIEWPORT_COLUMNS_MIN, VIEWPORT_COLUMNS_MAX)
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
    /// Full-trace X is non-decreasing (any viewport slice is then monotonic too).
    pub x_monotonic: bool,
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
                x_monotonic: true,
            },
        );
    }
    let max_columns = overview_column_count();
    let x_monotonic = n >= 2 && x_is_monotonic(x, n);
    let (columns, stats) = if x_monotonic {
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
        x_monotonic: n < 2 || x_monotonic,
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

    fn merge(&mut self, other: &Self) {
        if other.x_min.is_finite() {
            self.x_min = self.x_min.min(other.x_min);
            self.x_max = self.x_max.max(other.x_max);
        }
        if other.count > 0 {
            self.count += other.count;
            self.y_min = self.y_min.min(other.y_min);
            self.y_max = self.y_max.max(other.y_max);
            self.sum += other.sum;
            self.sum_sq += other.sum_sq;
        }
    }
}

fn vector_columns_with_stats(
    x: &[f64],
    y: &[f64],
    n: usize,
    mut stats: Option<&mut LoadPassStats>,
) -> Vec<ScopeEnvelopeColumn> {
    let mut out = Vec::with_capacity(n.saturating_sub(1).max(1));
    let mut last_finite: Option<(f64, f64)> = None;
    for i in 0..n {
        if let Some(s) = stats.as_mut() {
            s.add_sample(x[i], y[i]);
        }
        if x[i].is_finite() && y[i].is_finite() {
            if let Some((px, py)) = last_finite {
                out.push(segment_column(px, py, x[i], y[i]));
            }
            last_finite = Some((x[i], y[i]));
        } else {
            last_finite = None;
        }
    }
    if out.is_empty() {
        if let Some((px, py)) = last_finite {
            out.push(hold_column(px, px, py));
        }
    }
    out
}

fn decimate_envelope_index_buckets_with_stats(
    x: &[f64],
    y: &[f64],
    n: usize,
    max_columns: usize,
) -> (Vec<ScopeEnvelopeColumn>, LoadPassStats) {
    let mut stats = LoadPassStats::new();
    if n <= max_columns {
        let out = vector_columns_with_stats(x, y, n, Some(&mut stats));
        stats.finalize_bounds();
        return (out, stats);
    }
    let mut out = Vec::with_capacity(max_columns);
    for b in 0..max_columns {
        let start = b * n / max_columns;
        let end = ((b + 1) * n / max_columns).max(start);
        if start >= end {
            continue;
        }
        let x0 = x[start];
        let x1 = if end < n { x[end] } else { x[end - 1] };
        if let Some(col) = envelope_column_range_with_stats(x, y, start, end, x0, x1, &mut stats) {
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
    let mut stats = LoadPassStats::new();
    if n <= max_columns {
        let out = vector_columns_with_stats(x, y, n, Some(&mut stats));
        stats.finalize_bounds();
        return (out, stats);
    }
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
    let mut idx = 0usize;
    let mut hold: Option<f64> = None;
    for b in 0..max_columns {
        let t0 = x0 + span * (b as f64) / max_columns as f64;
        let t1 = if b + 1 == max_columns {
            x1
        } else {
            x0 + span * ((b + 1) as f64) / max_columns as f64
        };
        while idx < n && x[idx] < t0 {
            idx += 1;
        }
        let start = idx;
        if b + 1 == max_columns {
            while idx < n && x[idx] <= t1 {
                idx += 1;
            }
        } else {
            while idx < n && x[idx] < t1 {
                idx += 1;
            }
        }
        let end = idx;
        if start < end {
            if let Some(col) =
                envelope_column_range_with_stats(x, y, start, end, t0, t1, &mut stats)
            {
                hold = Some(col.y_last);
                out.push(col);
            } else {
                hold = None;
            }
        } else if let Some(y_hold) = hold {
            out.push(hold_column(t0, t1, y_hold));
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
    x0: f64,
    x1: f64,
    stats: &mut LoadPassStats,
) -> Option<ScopeEnvelopeColumn> {
    envelope_column_range(x, y, start, end, x0, x1, Some(stats))
}

/// Scope-style min/max envelope with explicit column time span (professional display).
pub fn decimate_envelope_columns(
    x: &[f64],
    y: &[f64],
    max_columns: usize,
) -> Vec<ScopeEnvelopeColumn> {
    decimate_envelope_columns_ex(x, y, max_columns, false)
}

fn decimate_envelope_columns_ex(
    x: &[f64],
    y: &[f64],
    max_columns: usize,
    assume_monotonic: bool,
) -> Vec<ScopeEnvelopeColumn> {
    let n = x.len().min(y.len());
    if n == 0 {
        return Vec::new();
    }
    let max_columns = max_columns.max(1);
    if n <= max_columns {
        return vector_columns_with_stats(x, y, n, None);
    }
    if assume_monotonic || (n >= 2 && x_is_monotonic(x, n)) {
        return decimate_envelope_time_buckets(x, y, n, max_columns);
    }
    decimate_envelope_index_buckets(x, y, n, max_columns)
}

#[inline]
fn x_is_monotonic(x: &[f64], n: usize) -> bool {
    x[..n]
        .windows(2)
        .all(|w| w[1] >= w[0] || (w[1] - w[0]).abs() < 1e-18)
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
        let end = ((b + 1) * n / max_columns).max(start);
        if start >= end {
            continue;
        }
        let x0 = x[start];
        let x1 = if end < n { x[end] } else { x[end - 1] };
        if let Some(col) = envelope_column_range(x, y, start, end, x0, x1, None) {
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
    let mut hold: Option<f64> = None;
    for b in 0..max_columns {
        let t0 = x0 + span * (b as f64) / max_columns as f64;
        let t1 = if b + 1 == max_columns {
            x1
        } else {
            x0 + span * ((b + 1) as f64) / max_columns as f64
        };
        while idx < n && x[idx] < t0 {
            idx += 1;
        }
        let start = idx;
        if b + 1 == max_columns {
            while idx < n && x[idx] <= t1 {
                idx += 1;
            }
        } else {
            while idx < n && x[idx] < t1 {
                idx += 1;
            }
        }
        let end = idx;
        if start < end {
            if let Some(col) = envelope_column_range(x, y, start, end, t0, t1, None) {
                hold = Some(col.y_last);
                out.push(col);
            } else {
                hold = None;
            }
        } else if let Some(y_hold) = hold {
            out.push(hold_column(t0, t1, y_hold));
        }
    }
    out
}

fn sample_time(x: &[f64], i: usize, fallback: f64) -> f64 {
    x.get(i)
        .copied()
        .filter(|v| v.is_finite())
        .unwrap_or(fallback)
}

fn envelope_column_range(
    x: &[f64],
    y: &[f64],
    start: usize,
    end: usize,
    x0: f64,
    x1: f64,
    mut stats: Option<&mut LoadPassStats>,
) -> Option<ScopeEnvelopeColumn> {
    if start >= end {
        return None;
    }
    let mut local = LoadPassStats::new();
    let mut y_first = None;
    let mut y_last = 0.0;
    let mut x_first = x0;
    let mut x_last = x1;
    let mut x_min = x0;
    let mut x_max = x0;
    let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut min_i = 0usize;
    let mut max_i = 0usize;
    let mut prev: Option<f64> = None;
    let mut dir = 0i8;
    let mut reversed = false;
    let mut any = false;
    for i in start..end {
        let v = y[i];
        local.add_sample(x[i], v);
        if !v.is_finite() {
            continue;
        }
        let t = sample_time(x, i, x0);
        if y_first.is_none() {
            y_first = Some(v);
            min_i = i;
            max_i = i;
            x_first = t;
            x_min = t;
            x_max = t;
        }
        y_last = v;
        x_last = t;
        if v < ymin {
            ymin = v;
            min_i = i;
            x_min = t;
        }
        if v > ymax {
            ymax = v;
            max_i = i;
            x_max = t;
        }
        if let Some(p) = prev {
            let d = if v > p {
                1
            } else if v < p {
                -1
            } else {
                0
            };
            if d != 0 {
                if dir != 0 && d != dir {
                    reversed = true;
                }
                dir = d;
            }
        }
        prev = Some(v);
        any = true;
    }
    let produced = y_first.filter(|_| any).map(|y_first| {
        let mut bx0 = x0;
        let mut bx1 = x1;
        if bx1 < bx0 {
            std::mem::swap(&mut bx0, &mut bx1);
        }
        ScopeEnvelopeColumn {
            x0: bx0,
            x1: bx1,
            y_min: ymin,
            y_max: ymax,
            y_first,
            y_last,
            x_first,
            x_last,
            x_min,
            x_max,
            min_before_max: min_i <= max_i,
            reversed,
        }
    });
    if let Some(s) = stats.as_mut() {
        s.merge(&local);
    }
    produced
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
        ymin = ymin.min(c.y_min).min(c.y_first).min(c.y_last);
        ymax = ymax.max(c.y_max).max(c.y_first).max(c.y_last);
    }
    if !xmin.is_finite() {
        return (0.0, 1.0, 0.0, 1.0);
    }
    (xmin, xmax, ymin, ymax)
}

/// Viewport LOD: Peak Detect envelope, or Vector samples when spp is low.
///
/// `prev_uniform` is the previous series mode for hysteresis (stay Vector until spp > 2.4).
pub fn build_viewport_series(
    x: &[f64],
    y: &[f64],
    x_view0: f64,
    x_view1: f64,
    plot_width_px: f32,
    prev_uniform: bool,
) -> WaveViewportSeries {
    build_viewport_series_ex(x, y, x_view0, x_view1, plot_width_px, prev_uniform, false)
}

/// Same LOD as [`build_viewport_series`]; `assume_monotonic` skips a redundant X scan.
pub fn build_viewport_series_ex(
    x: &[f64],
    y: &[f64],
    x_view0: f64,
    x_view1: f64,
    plot_width_px: f32,
    prev_uniform: bool,
    assume_monotonic: bool,
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
    let spp = vis as f64 / (plot_width_px.max(1.0) as f64);
    let use_vector = vis >= 2
        && vis <= UNIFORM_SAMPLE_CAP
        && if prev_uniform {
            spp < SPP_LEAVE_VECTOR
        } else {
            spp < SPP_ENTER_VECTOR
        };

    if use_vector {
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
    WaveViewportSeries::Envelope(Arc::new(decimate_envelope_columns_ex(
        xs,
        ys,
        cols,
        assume_monotonic,
    )))
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
        assert!(cols.iter().all(|c| (c.x1 - c.x0).abs() > 0.0));
    }

    #[test]
    fn square_wave_columns_have_spans() {
        let x: Vec<f64> = (0..1000).map(|i| i as f64 * 1e-6).collect();
        let y: Vec<f64> = x
            .iter()
            .map(|t| {
                if (*t * 1e6 as f64) as i32 % 200 < 100 {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        let cols = decimate_envelope_columns(&x, &y, 50);
        assert!(cols.len() >= 10);
        let has_edge = cols.iter().any(|c| !c.is_flat());
        let has_flat = cols.iter().any(|c| c.is_flat());
        assert!(has_edge && has_flat);
    }

    #[test]
    fn viewport_uniform_when_few_samples() {
        let x: Vec<f64> = (0..64).map(|i| i as f64).collect();
        let y: Vec<f64> = x
            .iter()
            .map(|v| if *v as i32 % 2 == 0 { 0.0 } else { 1.0 })
            .collect();
        match build_viewport_series(&x, &y, 0.0, 63.0, 800.0, false) {
            WaveViewportSeries::Uniform(p) => assert!(p.len() >= 32),
            WaveViewportSeries::Envelope(_) => panic!("expected uniform LOD for small window"),
        }
    }

    #[test]
    fn viewport_envelope_when_dense() {
        let n = 80_000;
        let x: Vec<f64> = (0..n).map(|i| i as f64 * 1e-6).collect();
        let y: Vec<f64> = x.iter().map(|t| t.sin()).collect();
        match build_viewport_series(&x, &y, 0.0, (n - 1) as f64 * 1e-6, 800.0, false) {
            WaveViewportSeries::Envelope(c) => {
                assert!(!c.is_empty());
                assert!(c.iter().all(|col| col.x1 >= col.x0));
            }
            WaveViewportSeries::Uniform(_) => panic!("expected Peak Detect envelope for high spp"),
        }
    }

    #[test]
    fn spp_hysteresis_stays_vector_until_leave() {
        let n = 1800;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let y = vec![0.0; n];
        // spp = 1800/800 = 2.25: enter Peak Detect from envelope, stay Vector if already Vector.
        match build_viewport_series(&x, &y, 0.0, (n - 1) as f64, 800.0, false) {
            WaveViewportSeries::Envelope(_) => {}
            WaveViewportSeries::Uniform(_) => panic!("spp 2.25 should enter Peak Detect"),
        }
        match build_viewport_series(&x, &y, 0.0, (n - 1) as f64, 800.0, true) {
            WaveViewportSeries::Uniform(p) => assert!(p.len() >= 2),
            WaveViewportSeries::Envelope(_) => panic!("hysteresis should keep Vector at spp 2.25"),
        }
    }

    #[test]
    fn assume_monotonic_matches_scanned_time_buckets() {
        let n = 8_000;
        let x: Vec<f64> = (0..n).map(|i| i as f64 * 1e-6).collect();
        let y: Vec<f64> = (0..n)
            .map(|i| (i as f64 * std::f64::consts::TAU / 250.0).sin())
            .collect();
        let scanned = decimate_envelope_columns(&x, &y, 200);
        let assumed = decimate_envelope_columns_ex(&x, &y, 200, true);
        assert_eq!(scanned.len(), assumed.len());
        for (a, b) in scanned.iter().zip(assumed.iter()) {
            assert_eq!(a.x0, b.x0);
            assert_eq!(a.x1, b.x1);
            assert_eq!(a.y_min, b.y_min);
            assert_eq!(a.y_max, b.y_max);
            assert_eq!(a.x_first, b.x_first);
            assert_eq!(a.x_min, b.x_min);
            assert_eq!(a.x_max, b.x_max);
            assert_eq!(a.x_last, b.x_last);
        }
        let snap = build_load_snapshot(&x, &y).1;
        assert!(snap.x_monotonic);
    }

    #[test]
    fn short_trace_overview_is_vector_not_dots() {
        let x: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let y = vec![1.0; 50];
        let cols = build_overview_envelope(&x, &y);
        assert!(cols.len() >= 2);
        assert!(cols.iter().all(|c| (c.x1 - c.x0).abs() > 0.5));
        assert!(cols.iter().all(|c| c.is_flat()));
    }

    #[test]
    fn ramp_columns_are_monotonic_endpoints() {
        let n = 10_000;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let y: Vec<f64> = x.clone();
        let cols = decimate_envelope_columns(&x, &y, 100);
        assert_eq!(cols.len(), 100);
        for c in &cols {
            assert!(c.x1 > c.x0);
            assert!(!c.has_interior_extrema());
            assert!(!c.reversed);
            assert!((c.y_first - c.y_min).abs() < 1e-9 || (c.y_last - c.y_min).abs() < 1e-9);
            assert!((c.y_first - c.y_max).abs() < 1e-9 || (c.y_last - c.y_max).abs() < 1e-9);
        }
    }

    #[test]
    fn empty_time_buckets_hold_last_value() {
        let mut x = Vec::new();
        let mut y = Vec::new();
        for i in 0..80 {
            x.push(i as f64 * 0.01);
            y.push(2.0);
        }
        for i in 0..80 {
            x.push(9.0 + i as f64 * 0.01);
            y.push(-3.0);
        }
        let cols = decimate_envelope_columns(&x, &y, 20);
        assert_eq!(cols.len(), 20);
        let mids: Vec<_> = cols
            .iter()
            .filter(|c| c.x0 > 1.5 && c.x1 < 8.5)
            .collect();
        assert!(!mids.is_empty());
        assert!(mids.iter().all(|c| (c.y_last - 2.0).abs() < 1e-9));
        assert!(mids.iter().all(|c| (c.x1 - c.x0).abs() > 0.0));
    }

    #[test]
    fn high_freq_bucket_keeps_interior_peak() {
        let n = 4000;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..n)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let cols = decimate_envelope_columns(&x, &y, 40);
        assert!(cols.iter().any(|c| c.needs_peak_detect()));
        assert!(cols.iter().any(|c| c.reversed));
        assert!(cols.iter().any(|c| (c.y_max - c.y_min).abs() > 1.5));
    }

    #[test]
    fn adjacent_columns_share_bucket_edges() {
        let n = 5000;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let y = vec![0.5; n];
        let cols = decimate_envelope_columns(&x, &y, 25);
        for w in cols.windows(2) {
            assert!((w[0].x1 - w[1].x0).abs() < 1e-9);
        }
    }

    #[test]
    fn interior_peak_keeps_actual_sample_time() {
        let n = 100;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let mut y = vec![0.0; n];
        y[25] = 10.0;
        let cols = decimate_envelope_columns(&x, &y, 10);
        let spike = cols
            .iter()
            .find(|c| c.y_max > 5.0)
            .expect("spike column");
        assert!(
            (spike.x_max - 25.0).abs() < 1e-9,
            "x_max should be sample time 25, got {}",
            spike.x_max
        );
        let pts = spike.mmfl_points();
        assert!(
            pts.iter()
                .any(|(px, py)| (*px - 25.0).abs() < 1e-9 && (*py - 10.0).abs() < 1e-9),
            "MMFL vertices should include the spike at t=25, got {pts:?}"
        );
        let span = spike.x1 - spike.x0;
        let fake_a = spike.x0 + span * 0.35;
        let fake_b = spike.x0 + span * 0.65;
        assert!(
            (25.0 - fake_a).abs() > 0.5 || (25.0 - fake_b).abs() > 0.5,
            "fixture must not land on the old 35/65 bucket fractions"
        );
    }
}
