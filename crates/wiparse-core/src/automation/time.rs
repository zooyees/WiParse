//! Clock correlation from observed synchronization pairs, never from host arrival time.
use super::contracts::{Quality, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncPoint {
    pub tick: String,
    pub reference_s: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingRequest {
    pub clock_id: String,
    pub reference_clock_id: String,
    pub boot_id: String,
    pub capture_session_id: String,
    pub tick_hz: u64,
    pub sync_method: String,
    pub pairs: Vec<SyncPoint>,
    pub sync_uncertainty_s: f64,
    pub max_residual_s: f64,
    pub quality: Quality,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeMapping {
    pub schema_version: String,
    pub clock_id: String,
    pub reference_clock_id: String,
    pub boot_id: String,
    pub capture_session_id: String,
    pub anchor_tick: String,
    pub anchor_reference_s: f64,
    pub seconds_per_tick: f64,
    pub valid_tick_end: String,
    pub drift_ppm: f64,
    pub residual_s: f64,
    pub uncertainty_s: f64,
    pub sync_method: String,
    pub quality: Quality,
}
pub fn fit(r: &MappingRequest) -> Result<TimeMapping, String> {
    if r.clock_id.is_empty()
        || r.reference_clock_id.is_empty()
        || r.clock_id == r.reference_clock_id
        || r.boot_id.is_empty()
        || r.capture_session_id.is_empty()
        || r.sync_method.is_empty()
        || r.tick_hz == 0
        || !(3..=4096).contains(&r.pairs.len())
    {
        return Err("TIME.INVALID_SYNC_IDENTITY_OR_COUNT".into());
    }
    if !r.sync_uncertainty_s.is_finite()
        || r.sync_uncertainty_s < 0.0
        || !r.max_residual_s.is_finite()
        || r.max_residual_s <= 0.0
    {
        return Err("TIME.INVALID_UNCERTAINTY".into());
    }
    let ticks: Vec<u64> = r
        .pairs
        .iter()
        .map(|p| p.tick.parse().map_err(|_| "TIME.INVALID_TICK".to_string()))
        .collect::<Result<_, _>>()?;
    if ticks.windows(2).any(|w| w[1] <= w[0])
        || r.pairs.iter().any(|p| !p.reference_s.is_finite())
        || r.pairs
            .windows(2)
            .any(|w| w[1].reference_s <= w[0].reference_s)
    {
        return Err("TIME.CLOCK_BREAK".into());
    }
    let anchor = ticks[0];
    // Absolute u64 ticks stay exact. Only the difference within a mapping is converted.
    if ticks[ticks.len() - 1] - anchor > 9_007_199_254_740_992 {
        return Err("TIME.MAPPING_SPAN_TOO_LARGE".into());
    }
    let n = ticks.len() as f64;
    let xs: Vec<f64> = ticks.iter().map(|t| (t - anchor) as f64).collect();
    let y0 = r.pairs[0].reference_s;
    let ys: Vec<f64> = r.pairs.iter().map(|p| p.reference_s - y0).collect();
    let xm = xs.iter().map(|x| x / n).sum::<f64>();
    let ym = ys.iter().map(|y| y / n).sum::<f64>();
    let slope = xs
        .iter()
        .zip(&ys)
        .map(|(x, y)| (x - xm) * (y - ym))
        .sum::<f64>()
        / xs.iter().map(|x| (x - xm).powi(2)).sum::<f64>();
    let intercept = ym - slope * xm;
    let residual = xs
        .iter()
        .zip(&ys)
        .map(|(x, y)| (y - intercept - slope * x).abs())
        .fold(0.0, f64::max);
    let reference = y0 + intercept;
    if !slope.is_finite() || slope <= 0.0 || !reference.is_finite() || !residual.is_finite() {
        return Err("TIME.INVALID_FIT".into());
    }
    let mut quality = r.quality.clone();
    if residual > r.max_residual_s {
        quality.flag("TIME.RESIDUAL_EXCEEDED");
    }
    let uncertainty = residual + r.sync_uncertainty_s + slope;
    if !uncertainty.is_finite() {
        return Err("TIME.INVALID_FIT".into());
    }
    Ok(TimeMapping {
        schema_version: SCHEMA_VERSION.into(),
        clock_id: r.clock_id.clone(),
        reference_clock_id: r.reference_clock_id.clone(),
        boot_id: r.boot_id.clone(),
        capture_session_id: r.capture_session_id.clone(),
        anchor_tick: anchor.to_string(),
        anchor_reference_s: reference,
        seconds_per_tick: slope,
        valid_tick_end: ticks.last().unwrap().to_string(),
        drift_ppm: (slope * r.tick_hz as f64 - 1.0) * 1e6,
        residual_s: residual,
        uncertainty_s: uncertainty,
        sync_method: r.sync_method.clone(),
        quality,
    })
}
pub fn convert(m: &TimeMapping, tick: &str, boot: &str, session: &str) -> Result<f64, String> {
    if m.schema_version != SCHEMA_VERSION
        || boot != m.boot_id
        || session != m.capture_session_id
        || !m.quality.valid()
    {
        return Err("TIME.INVALID_MAPPING_OR_SESSION".into());
    }
    let start = m
        .anchor_tick
        .parse::<u64>()
        .map_err(|_| "TIME.INVALID_TICK")?;
    let end = m
        .valid_tick_end
        .parse::<u64>()
        .map_err(|_| "TIME.INVALID_TICK")?;
    let t = tick.parse::<u64>().map_err(|_| "TIME.INVALID_TICK")?;
    if t < start || t > end || end - start > 9_007_199_254_740_992 {
        return Err("TIME.OUTSIDE_MAPPING".into());
    }
    if !m.seconds_per_tick.is_finite()
        || m.seconds_per_tick <= 0.0
        || !m.uncertainty_s.is_finite()
        || m.uncertainty_s < 0.0
    {
        return Err("TIME.INVALID_MAPPING".into());
    }
    let v = m.anchor_reference_s + (t - start) as f64 * m.seconds_per_tick;
    if !v.is_finite() {
        return Err("TIME.INVALID_MAPPING".into());
    }
    Ok(v)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> MappingRequest {
        let base = 9_007_199_254_740_993u64;
        MappingRequest {
            clock_id: "io".into(),
            reference_clock_id: "scope".into(),
            boot_id: "b".into(),
            capture_session_id: "s".into(),
            tick_hz: 1000,
            sync_method: "observed_hardware_edges".into(),
            pairs: (0..3)
                .map(|i| SyncPoint {
                    tick: (base + i * 1000).to_string(),
                    reference_s: 10.0 + i as f64,
                })
                .collect(),
            sync_uncertainty_s: 0.001,
            max_residual_s: 0.001,
            quality: Quality::default(),
        }
    }
    #[test]
    fn exact_large_ticks_no_extrapolation_or_session_reuse() {
        let m = fit(&request()).unwrap();
        assert_eq!(convert(&m, "9007199254741493", "b", "s").unwrap(), 10.5);
        assert!(convert(&m, "9007199254743993", "b", "s").is_err());
        assert!(convert(&m, "9007199254741493", "new", "s").is_err());
    }
    #[test]
    fn discontinuity_and_residual_gate() {
        let mut r = request();
        r.pairs[1].reference_s += 0.1;
        let m = fit(&r).unwrap();
        assert!(!m.quality.valid());
        assert!(convert(&m, &m.anchor_tick, "b", "s").is_err());
        r.pairs[1].tick = r.pairs[0].tick.clone();
        assert!(fit(&r).is_err());
    }
}
