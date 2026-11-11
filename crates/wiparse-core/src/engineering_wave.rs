//! Bounded, cancellable local waveform evidence. No certification or implicit limits.
use crate::instrument::WaveformTrace;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
#[derive(Default)]
struct Stats {
    n: u64,
    min: Option<f64>,
    max: Option<f64>,
    mean: f64,
    m2: f64,
}
impl Stats {
    fn add(&mut self, v: f64) {
        self.n += 1;
        self.min = Some(self.min.map_or(v, |x| x.min(v)));
        self.max = Some(self.max.map_or(v, |x| x.max(v)));
        let delta = v - self.mean;
        self.mean += delta / self.n as f64;
        self.m2 += delta * (v - self.mean);
    }
    fn value(&self) -> Value {
        json!({"count":self.n,"min":self.min,"max":self.max,"mean":(self.n>0).then_some(self.mean),"stddev":(self.n>0).then(||(self.m2/self.n as f64).max(0.).sqrt()),"peak_to_peak":self.min.zip(self.max).map(|(a,b)|b-a)})
    }
}
/// Stable identity of decoded samples, independent of file location or process.
pub fn trace_digest(trace: &WaveformTrace) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"wiparse-wave-trace-v1");
    for field in [&trace.channel, &trace.x_unit, &trace.y_unit] {
        hash.update((field.len() as u64).to_le_bytes());
        hash.update(field.as_bytes());
    }
    for axis in [&trace.x, &trace.y] {
        hash.update((axis.len() as u64).to_le_bytes());
        for chunk in axis.chunks(1024) {
            let mut bytes = [0u8; 8192];
            for (dest, sample) in bytes.chunks_exact_mut(8).zip(chunk) {
                dest.copy_from_slice(&sample.to_le_bytes());
            }
            hash.update(&bytes[..chunk.len() * 8]);
        }
    }
    format!("{:x}", hash.finalize())
}

/// All samples are scanned locally. Threshold and window are explicit physical inputs.
pub fn analyze(
    trace: &WaveformTrace,
    other: Option<&WaveformTrace>,
    p: &Value,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    if p.get("other_channel").is_some() && (other.is_none() || p["other_channel"]==p["channel"]) {return Err("WAVE.SECOND_CHANNEL_MISSING_OR_IDENTICAL".into());}
    let n = trace.x.len().min(trace.y.len());
    let a = p["start_s"].as_f64().unwrap_or(f64::NEG_INFINITY);
    let b = p["end_s"].as_f64().unwrap_or(f64::INFINITY);
    if trace.x_unit.trim() != "s" {
        return Err("WAVE.TIME_UNIT_MUST_BE_SECONDS".into());
    }
    let threshold = p["threshold"].as_f64();
    if threshold.is_some_and(|v| !v.is_finite()) {
        return Err("WAVE.INVALID_THRESHOLD".into());
    }
    if other.is_some() && (threshold.is_none() || p["other_threshold"].as_f64().is_none()) {
        return Err("WAVE.DEADTIME_REQUIRES_TWO_EXPLICIT_THRESHOLDS".into());
    }
    if let Some(o) = other {
        if o.x_unit != "s" || o.x.len() != trace.x.len() || o.y.len() != trace.y.len() {
            return Err("WAVE.CHANNELS_NOT_SYNCHRONIZED".into());
        }
    }
    let mut amplitude = Stats::default();
    let (mut scale, mut ssq) = (0f64, 0f64);
    let mut periods = Stats::default();
    let mut high = Stats::default();
    let mut low = Stats::default();
    let mut dead = Stats::default();
    let (mut last_t, mut last_y, mut rising, mut transition, mut high_state, mut dead_start) = (
        None::<f64>,
        None::<f64>,
        None::<f64>,
        None::<f64>,
        false,
        None::<f64>,
    );
    let mut invalid = 0u64;
    let mut selected = 0u64;
    let mut first_index=None;
    let mut last_index=None;
    let mut max_dt:Option<f64>=None;
    let mut min_dt = None::<f64>;
    let mut overlap = 0u64;
    let mut previous_leg=None;
    let mut first_to_second=Stats::default();
    let mut second_to_first=Stats::default();
    let mut same_leg_gaps=0u64;
    for i in 0..n {
        if i % 4096 == 0 && cancel.load(Ordering::Acquire) {
            return Err("AGENT.CANCELLED".into());
        }
        let (t, y) = (trace.x[i], trace.y[i]);
        if t.is_finite() && (t < a || t > b) {
            continue;
        }
        if !t.is_finite() || !y.is_finite() {
            invalid += 1;
            last_t = None;
            last_y = None;
            rising = None;
            transition = None;
            dead_start = None;
            previous_leg=None;
            continue;
        }
        if t < a || t > b {
            continue;
        }
        first_index.get_or_insert(i);
        last_index=Some(i);
        selected += 1;
        amplitude.add(y);
        let magnitude = y.abs();
        if magnitude > 0. {
            if magnitude > scale {
                ssq = 1. + ssq * (scale / magnitude).powi(2);
                scale = magnitude;
            } else {
                ssq += (magnitude / scale).powi(2);
            }
        }
        if let Some(prev) = last_t {
            if t <= prev {
                return Err("WAVE.TIME_AXIS_NOT_STRICTLY_INCREASING".into());
            }
            max_dt=Some(max_dt.map_or(t-prev,|d|d.max(t-prev)));
            min_dt = Some(min_dt.map_or(t - prev, |d| d.min(t - prev)));
        }
        if let Some(level) = threshold {
            let state = y >= level;
            if let (Some(prev_t), Some(prev_y)) = (last_t, last_y) {
                if state != high_state {
                    let edge = prev_t + (t - prev_t) * (level - prev_y) / (y - prev_y);
                    if let Some(previous) = transition {
                        if high_state {
                            high.add(edge - previous);
                        } else {
                            low.add(edge - previous);
                        }
                    }
                    transition = Some(edge);
                    if state {
                        if let Some(previous) = rising {
                            periods.add(edge - previous);
                        }
                        rising = Some(edge);
                    }
                }
            }
            high_state = state;
            if let Some(o) = other {
                let oy = o.y[i];
                if !o.x[i].is_finite() || !oy.is_finite() || (o.x[i] - t).abs() > 1e-12 {
                    return Err("WAVE.CHANNELS_NOT_SYNCHRONIZED_OR_INVALID".into());
                }
                let second = oy >= p["other_threshold"].as_f64().unwrap();
                match (state,second) {
                    (true,true)=>{overlap+=1;previous_leg=None;dead_start=None;}
                    (false,false)=>{if previous_leg.is_some() && dead_start.is_none(){dead_start=Some(t);}}
                    _=>{
                        if let (Some(start),Some(first))=(dead_start.take(),previous_leg) {
                            if first==state {same_leg_gaps+=1;} else {
                                let duration=t-start;
                                dead.add(duration);
                                if first {first_to_second.add(duration);}else{second_to_first.add(duration);}
                            }
                        }
                        previous_leg=Some(state);
                    }
                }
            }
        }
        last_t = Some(t);
        last_y = Some(y);
    }
    if amplitude.n == 0 {
        return Err("WAVE.NO_VALID_SAMPLES".into());
    }
    if threshold.is_none() {
        let mut auto = p.clone();
        auto["threshold"] = json!(amplitude.mean);
        let mut result = analyze(trace, other, &auto, cancel)?;
        result["threshold_source"] = json!(
            "estimated from valid sample mean; inspect waveform before using for logic timing"
        );
        return Ok(result);
    }
    Ok(
        json!({"first_sample_index":first_index,"last_sample_index":last_index,"sample_count":selected,"invalid_samples":invalid,"quality_complete":invalid==0,"requested_window_s":[p.get("start_s"),p.get("end_s")],"stage_label":p["stage"],"unit":trace.y_unit,"amplitude":amplitude.value(),"min":amplitude.min,"max":amplitude.max,"peak_to_peak":amplitude.min.zip(amplitude.max).map(|(a,b)|b-a),"mean":amplitude.mean,"rms":scale*(ssq/amplitude.n as f64).sqrt(),"threshold":threshold,"period_s":(periods.n>0).then_some(periods.mean),"period_statistics_s":periods.value(),"frequency_hz":(periods.n>0).then(||1./periods.mean),"high_width_s":high.value(),"low_width_s":low.value(),"dead_time_s":dead.value(),"first_to_second_dead_time_s":first_to_second.value(),"second_to_first_dead_time_s":second_to_first.value(),"same_leg_off_intervals_excluded":same_leg_gaps,"probe_deskew_verified":false,"simultaneous_high_samples":overlap,"sample_resolution_s":min_dt,"sample_interval_s":min_dt,"largest_observed_interval_s":max_dt,"external_alignment_verified":false,"raw_samples_uploaded":false,"quality_notes":"Timing uses explicit threshold crossings. Pulse statistics exclude incomplete end pulses. Dead time uses synchronized sample positions and has sample-resolution uncertainty; incomplete boundary off intervals excluded. Threshold uses native channel amplitude unit. Stage label is user supplied, not automatic Qi stage recognition. No product pass/fail without explicit limits."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn trace() -> WaveformTrace {
        WaveformTrace {
            channel: "CH1".into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
            x: WaveformTrace::wave_axis((0..1001).map(|n| n as f64 * 1e-6).collect()),
            y: WaveformTrace::wave_axis(
                (0..1001)
                    .map(|n| if n % 100 < 40 { 3.3 } else { 0. })
                    .collect(),
            ),
        }
    }
    #[test]
    fn pwm_period_width_and_cancellation() {
        let t = trace();
        let flag = AtomicBool::new(false);
        let r = analyze(&t, None, &json!({"threshold":1.65}), &flag).unwrap();
        assert!((r["frequency_hz"].as_f64().unwrap() - 10000.).abs() < 1.);
        assert!((r["high_width_s"]["mean"].as_f64().unwrap() - 40e-6).abs() < 1e-9);
        flag.store(true, Ordering::Release);
        assert!(analyze(&t, None, &json!({}), &flag).is_err());
    }
    #[test]
    fn empty_and_unsynchronized_are_errors() {
        let mut t = trace();
        let o = trace();
        t.x = WaveformTrace::wave_axis(Vec::new());
        assert!(analyze(&t, None, &json!({}), &AtomicBool::new(false)).is_err());
        assert!(analyze(
            &t,
            Some(&o),
            &json!({"threshold":1.,"other_threshold":1.}),
            &AtomicBool::new(false)
        )
        .is_err());
    }
}

#[cfg(test)]
mod quality_regressions {
    use super::*;
    fn square(n: usize, shift: usize) -> WaveformTrace {
        WaveformTrace {
            channel: "CH1".into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
            x: WaveformTrace::wave_axis((0..n).map(|i| i as f64 * 1e-6).collect()),
            y: WaveformTrace::wave_axis(
                (0..n)
                    .map(|i| if (i + shift) % 100 < 45 { 3.3 } else { 0. })
                    .collect(),
            ),
        }
    }
    #[test]
    fn synchronous_deadtime_is_quantized_and_wrong_timebase_is_rejected() {
        let t = square(1000, 0);
        let mut other = square(1000, 50);
        let flag = AtomicBool::new(false);
        let result = analyze(
            &t,
            Some(&other),
            &json!({"threshold":1.65,"other_threshold":1.65}),
            &flag,
        )
        .unwrap();
        assert!((result["dead_time_s"]["mean"].as_f64().unwrap() - 5e-6).abs() < 1e-12);
        assert_eq!(result["simultaneous_high_samples"], 0);
        other.x = WaveformTrace::wave_axis((0..1000).map(|i| i as f64 * 1e-6 + 1e-7).collect());
        assert!(analyze(
            &t,
            Some(&other),
            &json!({"threshold":1.65,"other_threshold":1.65}),
            &flag
        )
        .is_err());
    }
    #[test]
    fn constant_signal_has_no_invented_frequency() {
        let mut t = square(100, 0);
        t.y = WaveformTrace::wave_axis(vec![1.; 100]);
        let result = analyze(&t, None, &json!({}), &AtomicBool::new(false)).unwrap();
        assert!(analyze(&t,None,&json!({"channel":0,"other_channel":5,"threshold":1.,"other_threshold":1.}),&AtomicBool::new(false)).is_err());
        assert!(result["frequency_hz"].is_null());
        assert_eq!(result["period_statistics_s"]["count"], 0);
    }
    #[test]
    #[ignore = "manual synthetic resource baseline"]
    fn engineering_wave_resource_baseline() {
        let t = square(1_000_000, 0);
        let at = std::time::Instant::now();
        let result = analyze(
            &t,
            None,
            &json!({"threshold":1.65}),
            &AtomicBool::new(false),
        )
        .unwrap();
        let data = json!({"samples":1_000_000,"input_axis_bytes":16_000_000,"algorithm_additional_sample_arrays":0,"elapsed_ms":at.elapsed().as_millis(),"response_bytes":result.to_string().len(),"profile":if cfg!(debug_assertions){"debug"}else{"release"},"synthetic":true});
        eprintln!("{}", data);
        assert!(result.to_string().len() < 8192);
        if let Ok(path) = std::env::var("WIPARSE_AI_BASELINE_OUTPUT") {
            std::fs::write(path, data.to_string()).unwrap();
        }
    }
}
