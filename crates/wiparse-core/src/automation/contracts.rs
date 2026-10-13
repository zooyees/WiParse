use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "wiparse.automation/1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quality {
    #[serde(default)]
    pub issues: Vec<String>,
}
impl Quality {
    pub fn valid(&self) -> bool {
        self.issues.is_empty()
    }
    pub fn flag(&mut self, code: &str) {
        if !self.issues.iter().any(|s| s == code) {
            self.issues.push(code.into());
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    pub schema_version: String,
    pub metric_id: String,
    pub channel: String,
    pub value: Option<f64>,
    pub unit: String,
    pub sample_count: usize,
    pub window: Option<[f64; 2]>,
    pub uncertainty: Option<f64>,
    pub quality: Quality,
    pub algorithm_version: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}
impl Measurement {
    pub fn new(
        id: &str,
        channel: &str,
        value: Option<f64>,
        unit: &str,
        n: usize,
        quality: Quality,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.into(),
            metric_id: id.into(),
            channel: channel.into(),
            value: value.filter(|v| v.is_finite()),
            unit: unit.into(),
            sample_count: n,
            window: None,
            uncertainty: None,
            quality,
            algorithm_version: "1".into(),
            evidence_refs: vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    PASS,
    FAIL,
    INCONCLUSIVE,
    SKIP,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub limits_ref: String,
    pub limits_hash: String,
    pub approved: bool,
    pub unit: String,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub require_uncertainty: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub schema_version: String,
    pub metric_id: String,
    pub limits_ref: String,
    pub limits_hash: String,
    pub product_verdict: Verdict,
    pub reason_code: String,
    pub evidence_refs: Vec<String>,
}

pub fn evaluate(m: &Measurement, l: &Limits) -> CheckResult {
    let mut reason = "CHECK.PASS";
    let mut verdict = Verdict::PASS;
    let invalid_limits = !l.approved
        || l.limits_ref.is_empty()
        || l.limits_hash.is_empty()
        || l.minimum.is_none() && l.maximum.is_none()
        || l.minimum.is_some_and(|v| !v.is_finite())
        || l.maximum.is_some_and(|v| !v.is_finite())
        || l.minimum.zip(l.maximum).is_some_and(|(a, b)| a > b);
    if invalid_limits {
        reason = "CHECK.LIMITS_UNAPPROVED";
        verdict = Verdict::INCONCLUSIVE;
    } else if m.schema_version != SCHEMA_VERSION
        || m.metric_id.is_empty()
        || m.algorithm_version.is_empty()
    {
        reason = "CHECK.SCHEMA_INVALID";
        verdict = Verdict::INCONCLUSIVE;
    } else if m.unit != l.unit {
        reason = "CHECK.UNIT_MISMATCH";
        verdict = Verdict::INCONCLUSIVE;
    } else if !m.quality.valid() || m.sample_count == 0 || m.value.is_none() {
        reason = "CHECK.QUALITY_LOW";
        verdict = Verdict::INCONCLUSIVE;
    } else if let Some(v) = m.value {
        if !v.is_finite()
            || m.uncertainty.is_some_and(|u| !u.is_finite() || u < 0.0)
            || l.require_uncertainty && m.uncertainty.is_none()
        {
            reason = "CHECK.UNCERTAINTY_UNKNOWN";
            verdict = Verdict::INCONCLUSIVE;
        } else {
            let u = m.uncertainty.unwrap_or(0.0);
            if l.minimum.is_some_and(|lo| v + u < lo) || l.maximum.is_some_and(|hi| v - u > hi) {
                reason = "CHECK.OUT_OF_RANGE";
                verdict = Verdict::FAIL;
            } else if l.minimum.is_some_and(|lo| v - u < lo)
                || l.maximum.is_some_and(|hi| v + u > hi)
            {
                reason = "CHECK.GUARD_BAND";
                verdict = Verdict::INCONCLUSIVE;
            }
        }
    }
    CheckResult {
        schema_version: SCHEMA_VERSION.into(),
        metric_id: m.metric_id.clone(),
        limits_ref: l.limits_ref.clone(),
        limits_hash: l.limits_hash.clone(),
        product_verdict: verdict,
        reason_code: reason.into(),
        evidence_refs: m.evidence_refs.clone(),
    }
}
pub fn aggregate(required: &[Verdict], execution_completed: bool) -> Verdict {
    if required.contains(&Verdict::FAIL) {
        Verdict::FAIL
    } else if !execution_completed
        || required.is_empty()
        || required.iter().any(|v| *v != Verdict::PASS)
    {
        Verdict::INCONCLUSIVE
    } else {
        Verdict::PASS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> Limits {
        Limits {
            limits_ref: "vin".into(),
            limits_hash: "approved-hash".into(),
            approved: true,
            unit: "V".into(),
            minimum: Some(8.5),
            maximum: Some(9.5),
            require_uncertainty: true,
        }
    }
    #[test]
    fn missing_or_bad_data_cannot_pass() {
        let mut m = Measurement::new("vin", "CH1", Some(9.0), "V", 10, Quality::default());
        assert_eq!(
            evaluate(&m, &limits()).product_verdict,
            Verdict::INCONCLUSIVE
        );
        m.uncertainty = Some(0.01);
        assert_eq!(evaluate(&m, &limits()).product_verdict, Verdict::PASS);
        m.quality.flag("ACQ.OVERFLOW");
        assert_eq!(
            evaluate(&m, &limits()).product_verdict,
            Verdict::INCONCLUSIVE
        );
    }
    #[test]
    fn guard_band_and_fail_survive_execution_error() {
        let mut m = Measurement::new("vin", "CH1", Some(9.49), "V", 10, Quality::default());
        m.uncertainty = Some(0.02);
        assert_eq!(
            evaluate(&m, &limits()).product_verdict,
            Verdict::INCONCLUSIVE
        );
        m.value = Some(10.0);
        assert_eq!(evaluate(&m, &limits()).product_verdict, Verdict::FAIL);
        assert_eq!(aggregate(&[Verdict::FAIL], false), Verdict::FAIL);
        assert_eq!(aggregate(&[], true), Verdict::INCONCLUSIVE);
    }
}
