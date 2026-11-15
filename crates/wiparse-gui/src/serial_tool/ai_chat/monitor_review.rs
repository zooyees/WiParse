//! Deterministic helpers for an already frozen, bounded monitor sample.
//! These functions never read a live source or perform device operations.
use serde_json::{json, Value};

pub(super) fn is_local(method: &str) -> bool {
    matches!(method, "monitor.sample.search" | "monitor.math")
}

pub(super) fn execute(method: &str, params: &Value, sample: &Value) -> Result<Value, String> {
    let obj = params.as_object().ok_or("AI.MONITOR_LOCAL_INVALID")?;
    match method {
        "monitor.sample.search" => {
            if obj.keys().any(|k| !matches!(k.as_str(), "query" | "case_sensitive")) { return Err("AI.MONITOR_LOCAL_INVALID".into()); }
            let query = params["query"].as_str().filter(|s| !s.is_empty() && s.len() <= 256).ok_or("AI.MONITOR_LOCAL_INVALID")?;
            let case_sensitive = match params.get("case_sensitive") { None => false, Some(v) => v.as_bool().ok_or("AI.MONITOR_LOCAL_INVALID")? };
            let lines = sample["recent_lines"].as_array().ok_or("AI.MONITOR_SAMPLE_INVALID")?;
            let start = sample["window"]["sample_start_zero_based"].as_u64().ok_or("AI.MONITOR_SAMPLE_INVALID")?;
            let needle = if case_sensitive { query.to_owned() } else { query.to_lowercase() };
            let mut matches = Vec::new();
            let mut count = 0usize;
            for (offset, line) in lines.iter().enumerate() {
                let line = line.as_str().ok_or("AI.MONITOR_SAMPLE_INVALID")?;
                let found = if case_sensitive { line.contains(&needle) } else { line.to_lowercase().contains(&needle) };
                if found {
                    count += 1;
                    if matches.len() < 10 { matches.push(json!({"line_one_based":start.saturating_add(offset as u64).saturating_add(1),"text":line})); }
                }
            }
            Ok(json!({"ok":true,"data":{"matches":matches,"count":count,"examples_truncated":count > 10,"window":sample["window"],
                "coverage":"bounded sample only; lines may be truncated and omitted; not a complete log search"}}))
        }
        "monitor.math" => {
            if obj.keys().any(|k| !matches!(k.as_str(), "operation" | "a" | "b")) { return Err("AI.MONITOR_LOCAL_INVALID".into()); }
            let a = params["a"].as_f64().filter(|v|v.is_finite()).ok_or("AI.MONITOR_LOCAL_INVALID")?;
            let b = params["b"].as_f64().filter(|v|v.is_finite()).ok_or("AI.MONITOR_LOCAL_INVALID")?;
            let result = match params["operation"].as_str() {
                Some("add") => a+b, Some("subtract") => a-b, Some("multiply") => a*b,
                Some("divide") if b != 0.0 => a/b, _ => return Err("AI.MONITOR_LOCAL_INVALID".into()),
            };
            if !result.is_finite() { return Err("AI.MONITOR_LOCAL_NONFINITE".into()); }
            Ok(json!({"ok":true,"data":{"value":result,"input_provenance":"caller supplied; not an independent measurement"}}))
        }
        _ => Err("AI.MONITOR_LOCAL_INVALID".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_uses_frozen_rows_and_reports_bounded_coverage() {
        let sample=json!({"recent_lines":["VIN=5", "Error OVP"],"window":{"sample_start_zero_based":40,"end_exclusive_zero_based":42}});
        let result=execute("monitor.sample.search",&json!({"query":"error"}),&sample).unwrap();
        assert_eq!(result["data"]["count"],1);
        assert_eq!(result["data"]["matches"][0]["line_one_based"],42);
        assert!(result["data"]["coverage"].as_str().unwrap().contains("bounded"));
    }
    #[test]
    fn math_rejects_division_by_zero_and_nonfinite_results() {
        assert!(execute("monitor.math",&json!({"operation":"divide","a":1,"b":0}),&json!({})).is_err());
        assert!(execute("monitor.math",&json!({"operation":"multiply","a":1e308,"b":1e308}),&json!({})).is_err());
        assert_eq!(execute("monitor.math",&json!({"operation":"add","a":2,"b":3}),&json!({})).unwrap()["data"]["value"],5.0);
    }
}
