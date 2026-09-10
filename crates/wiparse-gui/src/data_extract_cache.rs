//! Disk cache for Data Analysis extracted series (`.wida`).
//!
//! Layout: magic + version + JSON header length + JSON + little-endian f64 pairs (x,y).

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use wiparse_core::paths::project_path;

const MAGIC: &[u8; 4] = b"WIDA";
const VERSION: u8 = 1;
const LIVE_FLUSH_EVERY: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
pub struct WidaSeriesMeta {
    pub label: String,
    pub kind: String,
    pub y_scale: f64,
    pub y_offset: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub n: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WidaHeader {
    pub source_key: String,
    pub content_fp: String,
    pub header: String,
    pub filters: Vec<String>,
    pub kinds: Vec<String>,
    pub frame_count: u32,
    pub series: Vec<WidaSeriesMeta>,
}

#[derive(Clone)]
pub struct WidaSeriesData {
    pub label: String,
    pub kind: String,
    pub y_scale: f64,
    pub y_offset: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub points: Vec<[f64; 2]>,
}

#[derive(Clone)]
pub struct WidaPayload {
    pub meta: WidaHeader,
    pub series: Vec<WidaSeriesData>,
}

pub fn cache_dir() -> PathBuf {
    project_path("cache/data_analysis")
}

pub fn ensure_cache_dir() -> std::io::Result<PathBuf> {
    let dir = cache_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn live_flush_interval() -> usize {
    LIVE_FLUSH_EVERY
}

/// Fingerprint for a source text file (path + len + mtime).
pub fn file_content_fp(path: &Path) -> String {
    let meta = fs::metadata(path).ok();
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}|{}|{}", path.to_string_lossy(), len, mtime)
}

pub fn rules_hash(header: &str, filters: &[String], kinds: &[String]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    header.hash(&mut h);
    filters.hash(&mut h);
    kinds.hash(&mut h);
    h.finish()
}

pub fn cache_key(content_fp: &str, rules: u64) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    content_fp.hash(&mut h);
    rules.hash(&mut h);
    format!("{:016x}", h.finish())
}

pub fn cache_path_for(content_fp: &str, header: &str, filters: &[String], kinds: &[String]) -> PathBuf {
    let key = cache_key(content_fp, rules_hash(header, filters, kinds));
    cache_dir().join(format!("{key}.wida"))
}

pub fn live_cache_path(session_id: &str) -> PathBuf {
    cache_dir().join(format!("live_{session_id}.wida"))
}

pub fn list_cached_extracts() -> Vec<(String, PathBuf)> {
    let Ok(dir) = fs::read_dir(cache_dir()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in dir.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wida") {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        out.push((name, path));
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out.truncate(40);
    out
}

pub fn save_wida(path: &Path, payload: &WidaPayload) -> Result<(), String> {
    save_wida_with_progress(path, payload, |_| {})
}

pub fn save_wida_with_progress<F>(
    path: &Path,
    payload: &WidaPayload,
    mut on_progress: F,
) -> Result<(), String>
where
    F: FnMut(f32),
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut meta = payload.meta.clone();
    meta.series = payload
        .series
        .iter()
        .map(|s| WidaSeriesMeta {
            label: s.label.clone(),
            kind: s.kind.clone(),
            y_scale: s.y_scale,
            y_offset: s.y_offset,
            y_min: s.y_min,
            y_max: s.y_max,
            n: s.points.len() as u32,
        })
        .collect();
    let json = serde_json::to_vec(&meta).map_err(|e| e.to_string())?;
    let mut file = File::create(path).map_err(|e| e.to_string())?;
    file.write_all(MAGIC).map_err(|e| e.to_string())?;
    file.write_all(&[VERSION]).map_err(|e| e.to_string())?;
    let len = json.len() as u32;
    file.write_all(&len.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&json).map_err(|e| e.to_string())?;

    let total_pts: usize = payload.series.iter().map(|s| s.points.len()).sum::<usize>().max(1);
    let mut written = 0usize;
    let report_every = (total_pts / 40).max(4096);
    for s in &payload.series {
        for p in &s.points {
            file.write_all(&p[0].to_le_bytes()).map_err(|e| e.to_string())?;
            file.write_all(&p[1].to_le_bytes()).map_err(|e| e.to_string())?;
            written += 1;
            if written % report_every == 0 || written == total_pts {
                on_progress(written as f32 / total_pts as f32);
            }
        }
    }
    on_progress(1.0);
    Ok(())
}

/// Write cache from borrowed point buffers (avoids cloning large series).
pub fn save_wida_from_parts(
    path: &Path,
    mut meta: WidaHeader,
    series: &[(String, String, f64, f64, f64, f64, &[ [f64; 2] ])],
    mut on_progress: impl FnMut(f32),
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    meta.series = series
        .iter()
        .map(|(label, kind, y_scale, y_offset, y_min, y_max, points)| WidaSeriesMeta {
            label: label.clone(),
            kind: kind.clone(),
            y_scale: *y_scale,
            y_offset: *y_offset,
            y_min: *y_min,
            y_max: *y_max,
            n: points.len() as u32,
        })
        .collect();
    let json = serde_json::to_vec(&meta).map_err(|e| e.to_string())?;
    let mut file = File::create(path).map_err(|e| e.to_string())?;
    file.write_all(MAGIC).map_err(|e| e.to_string())?;
    file.write_all(&[VERSION]).map_err(|e| e.to_string())?;
    let len = json.len() as u32;
    file.write_all(&len.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&json).map_err(|e| e.to_string())?;

    let total_pts: usize = series.iter().map(|s| s.6.len()).sum::<usize>().max(1);
    let mut written = 0usize;
    let report_every = (total_pts / 40).max(4096);
    for (_, _, _, _, _, _, points) in series {
        for p in *points {
            file.write_all(&p[0].to_le_bytes()).map_err(|e| e.to_string())?;
            file.write_all(&p[1].to_le_bytes()).map_err(|e| e.to_string())?;
            written += 1;
            if written % report_every == 0 || written == total_pts {
                on_progress(written as f32 / total_pts as f32);
            }
        }
    }
    on_progress(1.0);
    Ok(())
}

pub fn load_wida(path: &Path) -> Result<WidaPayload, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if &magic != MAGIC {
        return Err("not a WIDA cache".into());
    }
    let mut ver = [0u8; 1];
    file.read_exact(&mut ver).map_err(|e| e.to_string())?;
    if ver[0] != VERSION {
        return Err(format!("unsupported WIDA version {}", ver[0]));
    }
    let mut len_buf = [0u8; 4];
    file.read_exact(&mut len_buf).map_err(|e| e.to_string())?;
    let json_len = u32::from_le_bytes(len_buf) as usize;
    const MAX_WIDA_JSON: usize = 16 * 1024 * 1024;
    if json_len > MAX_WIDA_JSON {
        return Err(format!("WIDA header too large ({json_len} bytes)"));
    }
    let mut json = vec![0u8; json_len];
    file.read_exact(&mut json).map_err(|e| e.to_string())?;
    let meta: WidaHeader = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    let mut series = Vec::with_capacity(meta.series.len());
    for sm in &meta.series {
        let n = sm.n as usize;
        const MAX_SERIES_POINTS: usize = 50_000_000;
        if n > MAX_SERIES_POINTS {
            return Err(format!("WIDA series too large (n={n})"));
        }
        let mut raw = vec![0u8; n.saturating_mul(16)];
        file.read_exact(&mut raw).map_err(|e| e.to_string())?;
        let mut points = Vec::with_capacity(n);
        for chunk in raw.chunks_exact(16) {
            let xb: [u8; 8] = chunk[0..8].try_into().unwrap();
            let yb: [u8; 8] = chunk[8..16].try_into().unwrap();
            points.push([f64::from_le_bytes(xb), f64::from_le_bytes(yb)]);
        }
        series.push(WidaSeriesData {
            label: sm.label.clone(),
            kind: sm.kind.clone(),
            y_scale: sm.y_scale,
            y_offset: sm.y_offset,
            y_min: sm.y_min,
            y_max: sm.y_max,
            points,
        });
    }
    Ok(WidaPayload { meta, series })
}

/// Min/max envelope downsample for display-space points in [x0, x1].
pub fn downsample_minmax(points: &[[f64; 2]], x0: f64, x1: f64, cols: usize) -> Vec<[f64; 2]> {
    if points.is_empty() || cols == 0 {
        return Vec::new();
    }
    let span = (x1 - x0).abs().max(1e-12);
    let cols = cols.max(1);
    // Collect indices in range (points are usually sorted by x).
    let mut first = None;
    let mut last = None;
    for (i, p) in points.iter().enumerate() {
        if p[0] < x0 || p[0] > x1 {
            continue;
        }
        if first.is_none() {
            first = Some(i);
        }
        last = Some(i);
    }
    let (Some(a), Some(b)) = (first, last) else {
        return Vec::new();
    };
    let n = b - a + 1;
    if n <= cols.saturating_mul(3) {
        return points[a..=b].to_vec();
    }

    let mut buckets: Vec<(f64, f64, f64, f64, bool)> = vec![(0.0, 0.0, 0.0, 0.0, false); cols];
    // (x_min_pt, y_at_xmin, y_min, y_max, used) — emit min then max per column
    for p in &points[a..=b] {
        let t = ((p[0] - x0) / span).clamp(0.0, 0.999999);
        let bi = (t * cols as f64) as usize;
        let bkt = &mut buckets[bi.min(cols - 1)];
        if !bkt.4 {
            *bkt = (p[0], p[1], p[1], p[1], true);
        } else {
            if p[0] < bkt.0 {
                bkt.0 = p[0];
                bkt.1 = p[1];
            }
            bkt.2 = bkt.2.min(p[1]);
            bkt.3 = bkt.3.max(p[1]);
        }
    }

    let mut out = Vec::with_capacity(cols * 2);
    for bkt in buckets {
        if !bkt.4 {
            continue;
        }
        let (x, y_first, y_min, y_max, _) = bkt;
        if (y_max - y_min).abs() < 1e-15 {
            out.push([x, y_first]);
        } else {
            // Vertical extent in one column: min then max (or reverse by first).
            out.push([x, y_min]);
            out.push([x, y_max]);
        }
    }
    out
}
