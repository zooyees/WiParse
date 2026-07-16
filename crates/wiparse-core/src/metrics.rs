//! Electrical metrics frame: `AA55:<Vin_mV>:...:EDED`

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricSample {
    pub ts: f64,
    pub v_in: f64,
    pub i_in: f64,
    pub v_out: f64,
    pub i_out: f64,
    pub v_bat: f64,
    pub i_bat: f64,
    pub eff: f64,
    pub p: f64,
    pub t: i32,
    pub b: i32,
}

fn now_ts() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Parse `AA55:…:EDED` metrics line (same rules as Python `parse_metric_frame`).
pub fn parse_metric_frame(line: &str) -> Option<MetricSample> {
    let start = line.find("AA55")?;
    let end = line.find("EDED")?;
    if end < start {
        return None;
    }
    let slice = &line[start..end + 4];
    let parts: Vec<&str> = slice.split(':').collect();
    if parts.len() != 10 {
        return None;
    }
    let nums: Result<Vec<f64>, _> = parts[1..7].iter().map(|s| s.parse::<f64>()).collect();
    let nums = nums.ok()?;
    let v_in = nums[0] / 1000.0;
    let i_in = nums[1] / 1000.0;
    let v_out = nums[2] / 1000.0;
    let i_out = nums[3] / 1000.0;
    let v_bat = nums[4] / 1000.0;
    let i_bat = nums[5] / 1000.0;
    let p_out = v_out * i_out;
    let p_bat = v_bat * i_bat;
    let eff = if p_out > 0.1 {
        (p_bat / p_out * 100.0).min(100.0)
    } else {
        0.0
    };
    let t = parts[7].parse().ok()?;
    let b = parts[8].parse().ok()?;
    Some(MetricSample {
        ts: now_ts(),
        v_in,
        i_in,
        v_out,
        i_out,
        v_bat,
        i_bat,
        eff,
        p: p_out,
        t,
        b,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_demo_frame() {
        let m = parse_metric_frame("AA55:9000:1500:8500:1400:4000:3000:45:80:EDED").unwrap();
        assert!((m.v_in - 9.0).abs() < 1e-9);
        assert!((m.i_in - 1.5).abs() < 1e-9);
        assert!((m.v_bat - 4.0).abs() < 1e-9);
        assert_eq!(m.t, 45);
        assert_eq!(m.b, 80);
        assert!(m.eff > 0.0);
    }
}
