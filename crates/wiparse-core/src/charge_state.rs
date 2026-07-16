//! Simple charge-state estimator (CC / CV / trickle / idle).

use crate::metrics::MetricSample;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargePhase {
    Idle,
    Trickle,
    Cc,
    Cv,
    Unknown,
}

impl ChargePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Trickle => "trickle",
            Self::Cc => "cc",
            Self::Cv => "cv",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChargeStateTracker {
    window: VecDeque<(f64, f64)>, // (v_bat, i_bat)
    max_samples: usize,
    last: ChargePhase,
}

impl Default for ChargeStateTracker {
    fn default() -> Self {
        Self::new(40)
    }
}

impl ChargeStateTracker {
    pub fn new(max_samples: usize) -> Self {
        Self {
            window: VecDeque::with_capacity(max_samples),
            max_samples: max_samples.max(4),
            last: ChargePhase::Unknown,
        }
    }

    pub fn push(&mut self, m: &MetricSample) -> ChargePhase {
        if self.window.len() >= self.max_samples {
            self.window.pop_front();
        }
        self.window.push_back((m.v_bat, m.i_bat));
        self.last = self.estimate();
        self.last
    }

    pub fn current(&self) -> ChargePhase {
        self.last
    }

    fn estimate(&self) -> ChargePhase {
        if self.window.len() < 8 {
            return ChargePhase::Unknown;
        }
        let n = self.window.len() as f64;
        let i_avg: f64 = self.window.iter().map(|(_, i)| *i).sum::<f64>() / n;
        let v_avg: f64 = self.window.iter().map(|(v, _)| *v).sum::<f64>() / n;
        let v_first = self.window.front().map(|(v, _)| *v).unwrap_or(v_avg);
        let v_last = self.window.back().map(|(v, _)| *v).unwrap_or(v_avg);
        let dv = v_last - v_first;
        let i_first = self.window.front().map(|(_, i)| *i).unwrap_or(i_avg);
        let i_last = self.window.back().map(|(_, i)| *i).unwrap_or(i_avg);
        let di = i_last - i_first;

        if i_avg < 0.10 {
            ChargePhase::Idle
        } else if i_avg < 0.18 {
            ChargePhase::Trickle
        } else if dv > 0.025 && i_avg > 0.12 {
            ChargePhase::Cc
        } else if v_avg > 4.0 && di < -0.025 {
            ChargePhase::Cv
        } else if i_avg > 0.12 {
            ChargePhase::Cc
        } else {
            ChargePhase::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_when_current_low() {
        let mut t = ChargeStateTracker::new(20);
        for _ in 0..20 {
            let m = MetricSample {
                ts: 0.0,
                v_in: 9.0,
                i_in: 0.0,
                v_out: 5.0,
                i_out: 0.0,
                v_bat: 4.0,
                i_bat: 0.02,
                eff: 0.0,
                p: 0.0,
                t: 30,
                b: 80,
            };
            t.push(&m);
        }
        assert_eq!(t.current(), ChargePhase::Idle);
    }
}
