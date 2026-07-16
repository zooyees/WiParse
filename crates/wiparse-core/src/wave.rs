//! Electrical waveform helpers (live + session export).

use crate::db::DbError;
use crate::metrics::MetricSample;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

pub const DEFAULT_CHANNELS: &[&str] = &[
    "rel_t", "v_in", "i_in", "v_out", "i_out", "v_bat", "i_bat", "p",
];

fn channel_unit(ch: &str) -> &'static str {
    match ch {
        "rel_t" => "s",
        "v_in" | "v_out" | "v_bat" => "V",
        "i_in" | "i_out" | "i_bat" => "A",
        "p" => "W",
        "eff" => "%",
        "t" | "temp" => "C",
        "b" | "battery" => "%",
        _ => "",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricRow {
    pub rel_t: f64,
    pub v_in: f64,
    pub i_in: f64,
    pub v_out: f64,
    pub i_out: f64,
    pub v_bat: f64,
    pub i_bat: f64,
    pub eff: f64,
    pub p: f64,
    pub t: f64,
    pub b: f64,
}

impl From<&MetricSample> for MetricRow {
    fn from(m: &MetricSample) -> Self {
        Self {
            rel_t: 0.0,
            v_in: m.v_in,
            i_in: m.i_in,
            v_out: m.v_out,
            i_out: m.i_out,
            v_bat: m.v_bat,
            i_bat: m.i_bat,
            eff: m.eff,
            p: m.p,
            t: m.t as f64,
            b: m.b as f64,
        }
    }
}

impl MetricRow {
    fn get(&self, ch: &str) -> f64 {
        match ch {
            "rel_t" => self.rel_t,
            "v_in" => self.v_in,
            "i_in" => self.i_in,
            "v_out" => self.v_out,
            "i_out" => self.i_out,
            "v_bat" => self.v_bat,
            "i_bat" => self.i_bat,
            "eff" => self.eff,
            "p" | "power" => self.p,
            "t" | "temp" => self.t,
            "b" | "battery" => self.b,
            _ => 0.0,
        }
    }
}

pub fn metrics_to_wave(metrics: &[MetricRow], channels: &[&str]) -> Value {
    let mut chans: Vec<&str> = channels.to_vec();
    if !chans.iter().any(|c| *c == "rel_t") {
        chans.insert(0, "rel_t");
    }
    let points: Vec<Vec<f64>> = metrics
        .iter()
        .map(|m| chans.iter().map(|c| m.get(c)).collect())
        .collect();
    let sample_rate = if metrics.len() >= 2 {
        let dt = metrics[metrics.len() - 1].rel_t - metrics[0].rel_t;
        if dt > 0.0 {
            Some((((metrics.len() - 1) as f64) / dt * 1000.0).round() / 1000.0)
        } else {
            None
        }
    } else {
        None
    };
    let mut units = HashMap::new();
    for c in &chans {
        units.insert(*c, channel_unit(c));
    }
    json!({
        "sample_rate_hz_est": sample_rate,
        "channels": chans,
        "points": points,
        "units": units,
    })
}

pub fn fetch_session_metrics(
    conn: &Connection,
    session_id: i64,
    rel_from: Option<f64>,
    rel_to: Option<f64>,
) -> Result<Vec<MetricRow>, DbError> {
    let mut sql = String::from(
        "SELECT rel_time, v_in, i_in, v_out, i_out, v_bat, i_bat, eff, power, temp, battery
         FROM charging_metrics WHERE session_id=?1 ",
    );
    if rel_from.is_some() {
        sql.push_str("AND rel_time >= ?2 ");
    }
    if rel_to.is_some() {
        sql.push_str(if rel_from.is_some() {
            "AND rel_time <= ?3 "
        } else {
            "AND rel_time <= ?2 "
        });
    }
    sql.push_str("ORDER BY rel_time ASC");

    let mut stmt = conn.prepare(&sql)?;
    let map_row = |r: &rusqlite::Row| -> rusqlite::Result<MetricRow> {
        Ok(MetricRow {
            rel_t: r.get(0)?,
            v_in: r.get(1)?,
            i_in: r.get(2)?,
            v_out: r.get(3)?,
            i_out: r.get(4)?,
            v_bat: r.get(5)?,
            i_bat: r.get(6)?,
            eff: r.get(7)?,
            p: r.get(8)?,
            t: r.get(9)?,
            b: r.get(10)?,
        })
    };

    let rows = match (rel_from, rel_to) {
        (Some(a), Some(b)) => stmt.query_map(params![session_id, a, b], map_row)?,
        (Some(a), None) => stmt.query_map(params![session_id, a], map_row)?,
        (None, Some(b)) => stmt.query_map(params![session_id, b], map_row)?,
        (None, None) => stmt.query_map(params![session_id], map_row)?,
    };

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn export_metrics_csv(path: impl AsRef<Path>, metrics: &[MetricRow]) -> Result<usize, DbError> {
    let f = File::create(path.as_ref()).map_err(|e| DbError::Message(e.to_string()))?;
    let mut w = BufWriter::new(f);
    writeln!(w, "rel_t,v_in,i_in,v_out,i_out,v_bat,i_bat,eff,p,t,b")
        .map_err(|e| DbError::Message(e.to_string()))?;
    for m in metrics {
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{}",
            m.rel_t, m.v_in, m.i_in, m.v_out, m.i_out, m.v_bat, m.i_bat, m.eff, m.p, m.t, m.b
        )
        .map_err(|e| DbError::Message(e.to_string()))?;
    }
    Ok(metrics.len())
}

pub fn export_metrics_json(
    path: impl AsRef<Path>,
    metrics: &[MetricRow],
) -> Result<usize, DbError> {
    let text =
        serde_json::to_string_pretty(metrics).map_err(|e| DbError::Message(e.to_string()))?;
    std::fs::write(path, text).map_err(|e| DbError::Message(e.to_string()))?;
    Ok(metrics.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_shape() {
        let rows = vec![
            MetricRow {
                rel_t: 0.0,
                v_in: 9.0,
                i_in: 1.0,
                v_out: 8.0,
                i_out: 0.9,
                v_bat: 4.0,
                i_bat: 0.8,
                eff: 90.0,
                p: 7.2,
                t: 40.0,
                b: 50.0,
            },
            MetricRow {
                rel_t: 1.0,
                v_in: 9.1,
                i_in: 1.1,
                v_out: 8.1,
                i_out: 1.0,
                v_bat: 4.1,
                i_bat: 0.9,
                eff: 91.0,
                p: 8.1,
                t: 41.0,
                b: 51.0,
            },
        ];
        let w = metrics_to_wave(&rows, &["v_in", "i_in"]);
        assert_eq!(w["channels"][0], "rel_t");
        assert_eq!(w["points"].as_array().unwrap().len(), 2);
        assert!(w["sample_rate_hz_est"].as_f64().unwrap() > 0.0);
    }
}
