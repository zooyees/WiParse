use super::contracts::{Measurement, Quality};
use crate::instrument::WaveformTrace;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisOptions {
    pub window: Option<[f64; 2]>,
    pub threshold_v: Option<f64>,
    pub hysteresis_v: Option<f64>,
    pub burst_gap_s: Option<f64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct TraceStats {
    pub count: usize,
    pub dt: f64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
}
pub fn trace_stats(t: &WaveformTrace) -> Option<TraceStats> {
    if t.x.len() != t.y.len()
        || t.x.len() < 2
        || t.x.iter().chain(t.y.iter()).any(|v| !v.is_finite())
        || t.x.windows(2).any(|p| p[1] <= p[0])
    {
        return None;
    }
    let n = t.y.len();
    let min = t.y.iter().copied().fold(f64::INFINITY, f64::min);
    let max = t.y.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mean: f64 = t.y.iter().map(|v| v / n as f64).sum();
    if !mean.is_finite() {
        return None;
    }
    Some(TraceStats {
        count: n,
        dt: (t.x[n - 1] - t.x[0]) / (n - 1) as f64,
        min,
        max,
        mean,
    })
}
fn samples(t: &WaveformTrace, o: &AnalysisOptions) -> Result<Vec<(f64, f64)>, String> {
    if trace_stats(t).is_none() {
        return Err("ACQ.INVALID_SAMPLES".into());
    }
    let xs = match t.x_unit.trim() {
        "s" => 1.0,
        "ms" => 1e-3,
        "us" | "µs" => 1e-6,
        "ns" => 1e-9,
        _ => return Err("ACQ.TIME_UNIT".into()),
    };
    let ys = match t.y_unit.trim() {
        "V" => 1.0,
        "mV" => 1e-3,
        "uV" | "µV" => 1e-6,
        _ => return Err("ACQ.VOLTAGE_UNIT".into()),
    };
    if let Some([a, b]) = o.window {
        if !a.is_finite() || !b.is_finite() || a >= b {
            return Err("ACQ.INVALID_WINDOW".into());
        }
    }
    let s: Vec<_> =
        t.x.iter()
            .zip(t.y.iter())
            .map(|(x, y)| (x * xs, y * ys))
            .filter(|(x, _)| o.window.is_none_or(|[a, b]| *x >= a && *x <= b))
            .collect();
    if s.len() < 2 {
        return Err("ACQ.EMPTY_WINDOW".into());
    }
    Ok(s)
}
#[derive(Clone, Copy)]
struct Edge {
    t: f64,
    high: bool,
}
fn edges(s: &[(f64, f64)], o: &AnalysisOptions) -> Result<Vec<Edge>, String> {
    let threshold = o.threshold_v.ok_or("ANALYSIS.THRESHOLD_REQUIRED")?;
    let h = o.hysteresis_v.unwrap_or(0.0);
    if !threshold.is_finite() || !h.is_finite() || h < 0.0 {
        return Err("ANALYSIS.INVALID_THRESHOLD".into());
    }
    let mut high = s[0].1 > threshold;
    let mut es = vec![];
    for p in s.windows(2) {
        let next = if high {
            p[1].1 > threshold - h
        } else {
            p[1].1 >= threshold + h
        };
        if next != high {
            let level = if next { threshold + h } else { threshold - h };
            let alpha = ((level - p[0].1) / (p[1].1 - p[0].1)).clamp(0.0, 1.0);
            es.push(Edge {
                t: p[0].0 + alpha * (p[1].0 - p[0].0),
                high: next,
            });
            high = next;
        }
    }
    Ok(es)
}
pub fn analyze(
    t: &WaveformTrace,
    o: &AnalysisOptions,
    quality: &Quality,
) -> Result<Vec<Measurement>, String> {
    let s = samples(t, o)?;
    let n = s.len();
    let lo = s.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let hi = s.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let mean = s.iter().map(|p| p.1 / n as f64).sum();
    let mut out = vec![];
    for (id, val) in [
        ("voltage_min", lo),
        ("voltage_max", hi),
        ("voltage_pp", hi - lo),
        ("voltage_mean", mean),
    ] {
        let mut m = Measurement::new(id, &t.channel, Some(val), "V", n, quality.clone());
        m.window = Some([s[0].0, s[n - 1].0]);
        out.push(m);
    }
    if o.threshold_v.is_some() {
        let es = edges(&s, o)?;
        let rises: Vec<_> = es.iter().filter(|e| e.high).map(|e| e.t).collect();
        let gap = o.burst_gap_s.unwrap_or(f64::INFINITY);
        if o.burst_gap_s.is_some_and(|v| !v.is_finite() || v <= 0.0) {
            return Err("ANALYSIS.INVALID_BURST_GAP".into());
        }
        let periods: Vec<_> = rises
            .windows(2)
            .filter_map(|p| {
                let dt = p[1] - p[0];
                (dt > 0.0 && dt <= gap).then_some(dt)
            })
            .collect();
        let mut fs: Vec<_> = periods.iter().map(|dt| 1.0 / dt).collect();
        fs.sort_by(f64::total_cmp);
        let median = if fs.is_empty() {
            None
        } else if fs.len() % 2 == 0 {
            Some(fs[fs.len() / 2 - 1] / 2.0 + fs[fs.len() / 2] / 2.0)
        } else {
            Some(fs[fs.len() / 2])
        };
        for (id, value) in [
            ("frequency_min", fs.first().copied()),
            ("frequency_max", fs.last().copied()),
            ("frequency_median", median),
        ] {
            let mut q = quality.clone();
            if fs.is_empty() {
                q.flag("ANALYSIS.NO_CYCLES");
            }
            let mut m = Measurement::new(id, &t.channel, value, "Hz", fs.len(), q);
            m.window = Some([s[0].0, s[n - 1].0]);
            out.push(m);
        }
        let mut duty = Vec::new();
        for e in es.windows(3) {
            if e[0].high && !e[1].high && e[2].high {
                let period = e[2].t - e[0].t;
                if period > 0.0 && period <= gap {
                    duty.push((e[1].t - e[0].t) / period);
                }
            }
        }
        for (id, unit, v, count) in [
            (
                "period_min",
                "s",
                periods.iter().copied().reduce(f64::min),
                periods.len(),
            ),
            (
                "period_max",
                "s",
                periods.iter().copied().reduce(f64::max),
                periods.len(),
            ),
            (
                "duty_min",
                "1",
                duty.iter().copied().reduce(f64::min),
                duty.len(),
            ),
            (
                "duty_max",
                "1",
                duty.iter().copied().reduce(f64::max),
                duty.len(),
            ),
        ] {
            let mut q = quality.clone();
            if count == 0 {
                q.flag("ANALYSIS.NO_CYCLES");
            }
            let mut m = Measurement::new(id, &t.channel, v, unit, count, q);
            m.window = Some([s[0].0, s[n - 1].0]);
            out.push(m);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingProvenance {
    pub capture_id: String,
    pub raw: bool,
    pub synchronized: bool,
    pub deskew_verified: bool,
    pub high_deskew_s: f64,
    pub low_deskew_s: f64,
    pub uncertainty_s: f64,
    pub max_pair_gap_s: f64,
}
pub fn deadtime(
    h: &WaveformTrace,
    l: &WaveformTrace,
    o: &AnalysisOptions,
    p: &TimingProvenance,
    q: &Quality,
) -> Result<Vec<Measurement>, String> {
    let hs = samples(h, o)?;
    let ls = samples(l, o)?;
    if p.capture_id.is_empty() || !p.raw || !p.synchronized || !p.deskew_verified {
        return Err("ACQ.TIMING_PROVENANCE_REQUIRED".into());
    }
    if !p.uncertainty_s.is_finite()
        || p.uncertainty_s < 0.0
        || !p.high_deskew_s.is_finite()
        || !p.low_deskew_s.is_finite()
        || !p.max_pair_gap_s.is_finite()
        || p.max_pair_gap_s <= 0.0
    {
        return Err("ACQ.INVALID_TIMING_PROVENANCE".into());
    }
    if hs.len() != ls.len() || hs.iter().zip(&ls).any(|(a, b)| (a.0 - b.0).abs() > 1e-12) {
        return Err("ACQ.TIMEBASE_MISMATCH".into());
    }
    let he = edges(&hs, o)?;
    let le = edges(&ls, o)?;
    let mut out = vec![];
    // Each destination turn-on must pair with the latest source turn-off, without
    // a intervening source turn-on. Signed differences preserve overlap candidates.
    for (id, src, dst, sd, dd) in [
        ("deadtime_hl_min", &he, &le, p.high_deskew_s, p.low_deskew_s),
        ("deadtime_lh_min", &le, &he, p.low_deskew_s, p.high_deskew_s),
    ] {
        let mut used = std::collections::HashSet::new();
        let mut values = vec![];
        for (di, on) in dst.iter().enumerate().filter(|(_, e)| e.high) {
            // Capture-start edges have no observed destination off-state.
            if di == 0 || dst[di - 1].high {
                continue;
            }
            let split = src.partition_point(|e| e.t + sd < on.t + dd);
            let candidate = src
                .iter()
                .enumerate()
                .skip(split.saturating_sub(2))
                .take(4)
                .filter(|(_, e)| !e.high && (on.t + dd - e.t - sd).abs() <= p.max_pair_gap_s)
                .min_by(|(_, a), (_, b)| {
                    (on.t + dd - a.t - sd)
                        .abs()
                        .total_cmp(&(on.t + dd - b.t - sd).abs())
                });
            if let Some((i, off)) = candidate {
                let a = off.t + sd;
                let b = on.t + dd;
                let start = src.partition_point(|e| e.t + sd <= a.min(b));
                let wrong_cycle = src
                    .get(start)
                    .is_some_and(|e| e.high && e.t + sd < a.max(b))
                    || a <= dst[di - 1].t + dd;
                if !wrong_cycle && used.insert(i) {
                    values.push(b - a);
                }
            }
        }
        let mut quality = q.clone();
        if values.is_empty() {
            quality.flag("ANALYSIS.NO_MATCHED_COMMUTATION");
        }
        let max_dt = hs
            .windows(2)
            .chain(ls.windows(2))
            .map(|w| w[1].0 - w[0].0)
            .fold(0.0, f64::max);
        if p.uncertainty_s < max_dt {
            quality.flag("ACQ.UNCERTAINTY_BELOW_SAMPLE_INTERVAL");
        }
        let value = values.iter().copied().reduce(f64::min);
        let mut m = Measurement::new(
            id,
            &format!("{}:{}", h.channel, l.channel),
            value,
            "s",
            values.len(),
            quality,
        );
        m.uncertainty = Some(p.uncertainty_s);
        m.window = Some([hs[0].0, hs[hs.len() - 1].0]);
        m.evidence_refs.push(p.capture_id.clone());
        out.push(m);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wave(x: Vec<f64>, y: Vec<f64>) -> WaveformTrace {
        WaveformTrace {
            channel: "CH1".into(),
            x: x.into(),
            y: y.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        }
    }
    #[test]
    fn invalid_empty_and_units_rejected() {
        let t = wave(vec![], vec![]);
        assert!(analyze(&t, &AnalysisOptions::default(), &Quality::default()).is_err());
        let t = wave(vec![0.0, 0.0], vec![0.0, 1.0]);
        assert!(trace_stats(&t).is_none());
    }
    #[test]
    fn burst_idle_is_not_frequency() {
        let x = vec![0., 1., 2., 3., 4., 5., 20., 21., 22., 23.];
        let y = vec![0., 2., 0., 2., 0., 2., 0., 2., 0., 2.];
        let o = AnalysisOptions {
            threshold_v: Some(1.),
            burst_gap_s: Some(4.),
            ..Default::default()
        };
        let out = analyze(&wave(x, y), &o, &Quality::default()).unwrap();
        assert_eq!(
            out.iter()
                .find(|m| m.metric_id == "frequency_min")
                .unwrap()
                .value,
            Some(0.5)
        );
    }
    #[test]
    fn millivolts_normalize_and_quality_survives() {
        let mut t = wave(vec![0., 1.], vec![9000., 9100.]);
        t.y_unit = "mV".into();
        let mut q = Quality::default();
        q.flag("ACQ.OVERFLOW");
        let out = analyze(&t, &AnalysisOptions::default(), &q).unwrap();
        assert_eq!(out[0].value, Some(9.));
        assert!(!out[0].quality.valid());
    }
    #[test]
    fn deadtime_bidirectional_overlap_and_provenance() {
        let x: Vec<f64> = (0..400).map(|i| i as f64 * 1e-8).collect();
        let h = wave(
            x.clone(),
            (0..400)
                .map(|i| {
                    if (10..40).contains(&(i % 100)) {
                        3.0
                    } else {
                        0.0
                    }
                })
                .collect(),
        );
        let make_low = |start| {
            wave(
                x.clone(),
                (0..400)
                    .map(|i| {
                        if (start..90).contains(&(i % 100)) {
                            3.0
                        } else {
                            0.0
                        }
                    })
                    .collect(),
            )
        };
        let o = AnalysisOptions {
            threshold_v: Some(1.5),
            ..Default::default()
        };
        let mut p = TimingProvenance {
            capture_id: "capture".into(),
            raw: true,
            synchronized: true,
            deskew_verified: true,
            high_deskew_s: 0.,
            low_deskew_s: 0.,
            uncertainty_s: 2e-8,
            max_pair_gap_s: 2.5e-7,
        };
        let m = deadtime(&h, &make_low(50), &o, &p, &Quality::default()).unwrap();
        assert!((m[0].value.unwrap() - 1e-7).abs() < 1e-14);
        assert!((m[1].value.unwrap() - 2e-7).abs() < 1e-14);
        assert!(m.iter().all(|m| m.quality.valid()));
        let overlap = deadtime(&h, &make_low(35), &o, &p, &Quality::default()).unwrap();
        assert!((overlap[0].value.unwrap() + 5e-8).abs() < 1e-14);
        p.raw = false;
        assert!(deadtime(&h, &make_low(50), &o, &p, &Quality::default()).is_err());
    }
}
