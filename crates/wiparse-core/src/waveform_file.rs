//! Offline oscilloscope waveform file loaders and measurements.
//!
//! Supported:
//! - WiParse CSV export (`channel,index,x(unit),y(unit)`)
//! - Simple 2-column numeric CSV (`x,y` / `TIME,CH1`)
//! - Tektronix spreadsheet CSV (metadata header + TIME/CHx columns)
//! - Tektronix ISF (WFMPRE ASCII preamble + CURVE binary block)
//! - Tektronix WFM#001 (Windows reference waveform, YT INT16)

use crate::instrument::WaveformTrace;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WaveformFileError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Parse(String),
}

#[derive(Debug, Clone)]
pub struct WaveformMeasurements {
    pub count: usize,
    pub dt: f64,
    pub min: f64,
    pub max: f64,
    pub pp: f64,
    pub mean: f64,
    pub rms: f64,
    /// Estimated frequency from zero-crossings (Hz). `None` if not enough edges.
    pub freq_hz: Option<f64>,
    pub period: Option<f64>,
}

/// Load a waveform source file. Format is inferred from extension / content.
pub fn load_waveform_file(path: impl AsRef<Path>) -> Result<WaveformTrace, WaveformFileError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("CH1")
        .to_string();

    match ext.as_str() {
        "isf" => load_tek_isf(&bytes, &stem),
        "wfm" => load_tek_wfm(&bytes, &stem),
        "csv" | "txt" => load_waveform_csv_bytes(&bytes, &stem),
        _ => {
            if looks_like_wfm(&bytes) {
                load_tek_wfm(&bytes, &stem)
            } else if looks_like_isf(&bytes) {
                load_tek_isf(&bytes, &stem)
            } else {
                load_waveform_csv_bytes(&bytes, &stem)
            }
        }
    }
}

/// Load waveform bytes with an optional extension hint (`isf` / `wfm` / `csv`).
pub fn load_waveform_bytes(
    bytes: &[u8],
    hint_ext: &str,
    default_channel: &str,
) -> Result<WaveformTrace, WaveformFileError> {
    let ext = hint_ext.trim().trim_start_matches('.').to_ascii_lowercase();
    match ext.as_str() {
        "isf" => load_tek_isf(bytes, default_channel),
        "wfm" => load_tek_wfm(bytes, default_channel),
        "csv" | "txt" => load_waveform_csv_bytes(bytes, default_channel),
        _ => {
            if looks_like_wfm(bytes) {
                load_tek_wfm(bytes, default_channel)
            } else if looks_like_isf(bytes) {
                load_tek_isf(bytes, default_channel)
            } else {
                load_waveform_csv_bytes(bytes, default_channel)
            }
        }
    }
}

/// Save native instrument bytes or convert a parsed trace to `.isf` / `.wfm` / `.csv`.
pub fn save_waveform_file(
    path: impl AsRef<Path>,
    native_bytes: Option<&[u8]>,
    native_ext: Option<&str>,
    trace: Option<&WaveformTrace>,
) -> Result<(), WaveformFileError> {
    let path = path.as_ref();
    let target = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("csv")
        .to_ascii_lowercase();
    let native_ext = native_ext
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .unwrap_or_default();

    if let Some(bytes) = native_bytes {
        if !native_ext.is_empty() && native_ext == target {
            std::fs::write(path, bytes)?;
            return Ok(());
        }
    }

    let trace = if let Some(t) = trace {
        t.clone()
    } else if let Some(bytes) = native_bytes {
        load_waveform_bytes(bytes, &native_ext, "CH1")?
    } else {
        return Err(WaveformFileError::Parse(
            "no waveform data to save".into(),
        ));
    };

    match target.as_str() {
        "isf" => export_waveform_isf(path, &trace),
        "wfm" => export_waveform_wfm(path, &trace),
        "csv" | "txt" => export_waveform_csv(path, &trace),
        _ => export_waveform_csv(path, &trace),
    }
}

pub fn export_waveform_csv(
    path: impl AsRef<Path>,
    trace: &WaveformTrace,
) -> Result<(), WaveformFileError> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    writeln!(
        file,
        "channel,index,x({}),y({})",
        trace.x_unit, trace.y_unit
    )?;
    let n = trace.x.len().min(trace.y.len());
    for i in 0..n {
        writeln!(
            file,
            "{},{},{},{}",
            csv_cell(&trace.channel),
            i,
            trace.x[i],
            trace.y[i]
        )?;
    }
    Ok(())
}

/// Export trace as Tektronix ISF (WFMPRE + CURVE block).
pub fn export_waveform_isf(
    path: impl AsRef<Path>,
    trace: &WaveformTrace,
) -> Result<(), WaveformFileError> {
    let n = trace.x.len().min(trace.y.len());
    if n == 0 {
        return Err(WaveformFileError::Parse("empty waveform".into()));
    }
    let dt = if n >= 2 {
        (trace.x[n - 1] - trace.x[0]) / (n as f64 - 1.0)
    } else {
        1.0
    };
    let xzero = trace.x.first().copied().unwrap_or(0.0);
    let (ymin, ymax) = trace.y[..n].iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
        (lo.min(v), hi.max(v))
    });
    let yrange = (ymax - ymin).max(1e-12);
    let ymult = yrange / 250.0;
    let yoff = 128.0;
    let yzero = ymin;

    let mut curve = Vec::with_capacity(n);
    for &yv in &trace.y[..n] {
        let code = ((yv - yzero) / ymult + yoff).round().clamp(-128.0, 127.0) as i8;
        curve.push(code as u8);
    }

    let preamble = format!(
        ":WFMPRE:BYT_NR 1;BIT_NR 8;ENCDG BIN;BN_FMT RI;BYT_OR MSB;NR_PT {n};\
         WFID \"{}\";PT_FMT Y;XINCR {dt:.12E};PT_OFF 0;XZERO {xzero:.12E};XUNIT \"{}\";\
         YMULT {ymult:.12E};YZERO {yzero:.12E};YOFF {yoff:.12E};YUNIT \"{}\";",
        trace.channel, trace.x_unit, trace.y_unit
    );
    let len = curve.len();
    let mut out = preamble.into_bytes();
    out.extend_from_slice(format!(":CURVE #{}{len}", len.to_string().len()).as_bytes());
    out.extend_from_slice(len.to_string().as_bytes());
    out.extend_from_slice(&curve);
    std::fs::write(path, out)?;
    Ok(())
}

/// Export trace as Tektronix WFM#001 (YT INT16, single frame).
pub fn export_waveform_wfm(
    path: impl AsRef<Path>,
    trace: &WaveformTrace,
) -> Result<(), WaveformFileError> {
    let n = trace.x.len().min(trace.y.len());
    if n == 0 {
        return Err(WaveformFileError::Parse("empty waveform".into()));
    }
    let dt = if n >= 2 {
        (trace.x[n - 1] - trace.x[0]) / (n as f64 - 1.0)
    } else {
        1.0
    };
    let xzero = trace.x.first().copied().unwrap_or(0.0);
    let (ymin, ymax) = trace.y[..n].iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
        (lo.min(v), hi.max(v))
    });
    let yrange = (ymax - ymin).max(1e-12);
    let v_scale = yrange / 65534.0;
    let v_offset = ymin;
    let pre = 16usize;
    let post = 16usize;
    let record = n + pre + post;
    let bytes_per_pt = 2usize;
    let curve_bytes = record * bytes_per_pt;
    let header_len = 820usize;
    let file_len = header_len + curve_bytes + 8;

    let mut buf = vec![0u8; file_len];
    // Intel byte order marker
    buf[0] = 0x0f;
    buf[1] = 0x0f;
    buf[2..10].copy_from_slice(b"WFM#001\0");
    buf[10] = 9; // digits in EOF count
    buf[11..15].copy_from_slice(&(file_len as i32).to_le_bytes());
    buf[15] = bytes_per_pt as u8;
    buf[16..20].copy_from_slice(&(header_len as i32).to_le_bytes());

    write_f64_le(&mut buf, 174, v_scale);
    write_f64_le(&mut buf, 182, v_offset);
    write_u32_le(&mut buf, 186, 65532);
    write_f64_le(&mut buf, 478, dt);
    write_f64_le(&mut buf, 486, xzero);
    write_u32_le(&mut buf, 494, record as u32);
    write_i32_le(&mut buf, 238, 0); // EXPLICIT_INT16

    let data_start = pre * bytes_per_pt;
    let post_start = (pre + n) * bytes_per_pt;
    write_u32_le(&mut buf, 804, data_start as u32);
    write_u32_le(&mut buf, 808, post_start as u32);
    write_u32_le(&mut buf, 812, curve_bytes as u32);
    write_u32_le(&mut buf, 816, curve_bytes as u32);

    for (i, &yv) in trace.y[..n].iter().enumerate() {
        let code = ((yv - v_offset) / v_scale).round().clamp(-32768.0, 32767.0) as i16;
        let off = header_len + (pre + i) * bytes_per_pt;
        buf[off..off + 2].copy_from_slice(&code.to_le_bytes());
    }

    std::fs::write(path, buf)?;
    Ok(())
}

fn write_f64_le(buf: &mut [u8], off: usize, v: f64) {
    if off + 8 <= buf.len() {
        buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }
}

fn write_i32_le(buf: &mut [u8], off: usize, v: i32) {
    if off + 4 <= buf.len() {
        buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
}

fn write_u32_le(buf: &mut [u8], off: usize, v: u32) {
    if off + 4 <= buf.len() {
        buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
}

fn looks_like_wfm(bytes: &[u8]) -> bool {
    bytes.len() >= 9 && bytes[2..].starts_with(b"WFM#00")
}

fn looks_like_csv(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
    head.contains(',') && (head.contains("TIME") || head.contains("channel") || head.contains("CH"))
}

pub fn measure_waveform(trace: &WaveformTrace) -> WaveformMeasurements {
    let n = trace.x.len().min(trace.y.len());
    measure_samples(&trace.x[..n], &trace.y[..n])
}

/// Measure samples whose X falls in `[x0, x1]` (order-independent).
pub fn measure_waveform_range(
    trace: &WaveformTrace,
    x0: f64,
    x1: f64,
) -> WaveformMeasurements {
    let n = trace.x.len().min(trace.y.len());
    if n == 0 {
        return measure_samples(&[], &[]);
    }
    let (lo, hi) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for i in 0..n {
        let x = trace.x[i];
        if x >= lo && x <= hi {
            xs.push(x);
            ys.push(trace.y[i]);
        }
    }
    measure_samples(&xs, &ys)
}

fn measure_samples(x: &[f64], y: &[f64]) -> WaveformMeasurements {
    let n = x.len().min(y.len());
    if n == 0 {
        return WaveformMeasurements {
            count: 0,
            dt: 0.0,
            min: 0.0,
            max: 0.0,
            pp: 0.0,
            mean: 0.0,
            rms: 0.0,
            freq_hz: None,
            period: None,
        };
    }

    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut sum = 0.0;
    let mut sum_sq = 0.0;
    for &yv in &y[..n] {
        min = min.min(yv);
        max = max.max(yv);
        sum += yv;
        sum_sq += yv * yv;
    }
    let mean = sum / n as f64;
    let rms = (sum_sq / n as f64).sqrt();
    let dt = if n >= 2 {
        (x[n - 1] - x[0]) / (n as f64 - 1.0)
    } else {
        0.0
    };

    let (freq_hz, period) = estimate_frequency(&x[..n], &y[..n], mean);

    WaveformMeasurements {
        count: n,
        dt,
        min,
        max,
        pp: max - min,
        mean,
        rms,
        freq_hz,
        period,
    }
}

fn estimate_frequency(x: &[f64], y: &[f64], mean: f64) -> (Option<f64>, Option<f64>) {
    if x.len() < 4 {
        return (None, None);
    }
    let mut crossings = Vec::new();
    for i in 1..y.len() {
        let y0 = y[i - 1] - mean;
        let y1 = y[i] - mean;
        if y0 <= 0.0 && y1 > 0.0 {
            let denom = y1 - y0;
            let frac = if denom.abs() > f64::EPSILON {
                (-y0) / denom
            } else {
                0.0
            };
            crossings.push(x[i - 1] + frac * (x[i] - x[i - 1]));
        }
    }
    if crossings.len() < 2 {
        return (None, None);
    }
    let period = (crossings[crossings.len() - 1] - crossings[0]) / (crossings.len() - 1) as f64;
    if period <= 0.0 || !period.is_finite() {
        return (None, None);
    }
    (Some(1.0 / period), Some(period))
}

fn looks_like_isf(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(64)]).to_ascii_uppercase();
    head.contains("WFMP") || head.contains(":CURVE")
}

fn load_waveform_csv_bytes(bytes: &[u8], default_channel: &str) -> Result<WaveformTrace, WaveformFileError> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(WaveformFileError::Parse("empty CSV".into()));
    }

    // WiParse export: channel,index,x(unit),y(unit)
    if let Some(trace) = try_parse_wiparse_csv(&lines)? {
        return Ok(trace);
    }

    // Tek / generic spreadsheet: find a header row with TIME + channel, then numeric pairs.
    if let Some(trace) = try_parse_spreadsheet_csv(&lines, default_channel)? {
        return Ok(trace);
    }

    // Fallback: first two numeric columns on every row.
    try_parse_numeric_pairs(&lines, default_channel)
}

fn try_parse_wiparse_csv(lines: &[&str]) -> Result<Option<WaveformTrace>, WaveformFileError> {
    let header = lines[0].to_ascii_lowercase();
    if !(header.starts_with("channel,") && header.contains("index,")) {
        return Ok(None);
    }
    let (x_unit, y_unit) = parse_wiparse_units(lines[0]);
    let mut channel = String::new();
    let mut x = Vec::new();
    let mut y = Vec::new();
    for line in &lines[1..] {
        let cols = split_csv(line);
        if cols.len() < 4 {
            continue;
        }
        if channel.is_empty() {
            channel = cols[0].clone();
        }
        let xv: f64 = cols[2]
            .parse()
            .map_err(|_| WaveformFileError::Parse(format!("bad x value: {}", cols[2])))?;
        let yv: f64 = cols[3]
            .parse()
            .map_err(|_| WaveformFileError::Parse(format!("bad y value: {}", cols[3])))?;
        x.push(xv);
        y.push(yv);
    }
    if x.is_empty() {
        return Err(WaveformFileError::Parse("WiParse CSV has no samples".into()));
    }
    if channel.is_empty() {
        channel = "CH1".into();
    }
    Ok(Some(WaveformTrace {
        channel,
        x,
        y,
        x_unit,
        y_unit,
    }))
}

fn parse_wiparse_units(header: &str) -> (String, String) {
    let mut x_unit = "s".to_string();
    let mut y_unit = "V".to_string();
    for part in header.split(',') {
        let p = part.trim();
        if let Some(inner) = p.strip_prefix("x(").and_then(|s| s.strip_suffix(')')) {
            x_unit = inner.to_string();
        }
        if let Some(inner) = p.strip_prefix("y(").and_then(|s| s.strip_suffix(')')) {
            y_unit = inner.to_string();
        }
    }
    (x_unit, y_unit)
}

fn try_parse_spreadsheet_csv(
    lines: &[&str],
    default_channel: &str,
) -> Result<Option<WaveformTrace>, WaveformFileError> {
    let mut header_idx = None;
    let mut x_col = 0usize;
    let mut y_col = 1usize;
    let mut channel = default_channel.to_string();
    let mut x_unit = "s".to_string();
    let mut y_unit = "V".to_string();

    for (i, line) in lines.iter().enumerate() {
        let cols = split_csv(line);
        if cols.len() < 2 {
            continue;
        }
        let c0 = cols[0].to_ascii_lowercase();
        let looks_time = c0 == "time" || c0 == "t" || c0.starts_with("time(") || c0 == "x";
        if !looks_time {
            continue;
        }
        let mut found_y = None;
        for (ci, c) in cols.iter().enumerate().skip(1) {
            let u = c.to_ascii_uppercase();
            if u.starts_with("CH") || u.contains("VOLT") || u == "Y" {
                found_y = Some(ci);
                channel = sanitize_channel_name(c);
                if let Some(unit) = unit_in_parens(c) {
                    y_unit = unit;
                }
                break;
            }
        }
        header_idx = Some(i);
        x_col = 0;
        y_col = found_y.unwrap_or(1);
        if let Some(unit) = unit_in_parens(&cols[0]) {
            x_unit = unit;
        }
        if found_y.is_none() && cols.len() > 1 {
            channel = sanitize_channel_name(&cols[1]);
            if let Some(unit) = unit_in_parens(&cols[1]) {
                y_unit = unit;
            }
        }
        break;
    }

    let Some(start) = header_idx else {
        return Ok(None);
    };

    let mut x = Vec::new();
    let mut y = Vec::new();
    for line in &lines[start + 1..] {
        let cols = split_csv(line);
        if cols.len() <= y_col.max(x_col) {
            continue;
        }
        let Ok(xv) = cols[x_col].parse::<f64>() else {
            continue;
        };
        let Ok(yv) = cols[y_col].parse::<f64>() else {
            continue;
        };
        x.push(xv);
        y.push(yv);
    }
    if x.is_empty() {
        return Err(WaveformFileError::Parse(
            "spreadsheet CSV has no numeric samples".into(),
        ));
    }
    Ok(Some(WaveformTrace {
        channel,
        x,
        y,
        x_unit,
        y_unit,
    }))
}

fn try_parse_numeric_pairs(
    lines: &[&str],
    default_channel: &str,
) -> Result<WaveformTrace, WaveformFileError> {
    let mut x = Vec::new();
    let mut y = Vec::new();
    for line in lines {
        if line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let cols = split_csv(line);
        if cols.len() < 2 {
            continue;
        }
        let c0 = cols[0].to_ascii_lowercase();
        if c0 == "time" || c0 == "channel" || c0 == "index" {
            continue;
        }
        let Ok(xv) = cols[0].parse::<f64>() else {
            continue;
        };
        let Ok(yv) = cols[1].parse::<f64>() else {
            continue;
        };
        x.push(xv);
        y.push(yv);
    }
    if x.is_empty() {
        return Err(WaveformFileError::Parse(
            "CSV has no numeric x,y samples".into(),
        ));
    }
    Ok(WaveformTrace {
        channel: default_channel.to_string(),
        x,
        y,
        x_unit: "s".into(),
        y_unit: "V".into(),
    })
}

fn unit_in_parens(label: &str) -> Option<String> {
    let start = label.find('(')?;
    let end = label[start + 1..].find(')')?;
    let unit = label[start + 1..start + 1 + end].trim();
    if unit.is_empty() {
        None
    } else {
        Some(unit.to_string())
    }
}

fn sanitize_channel_name(raw: &str) -> String {
    let base = raw.split('(').next().unwrap_or(raw).trim();
    if base.is_empty() {
        "CH1".into()
    } else {
        base.to_string()
    }
}

fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for ch in line.chars() {
        match ch {
            '"' => in_q = !in_q,
            ',' if !in_q => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// Parse Tektronix WFM#001 (Windows reference waveform, YT INT16).
fn load_tek_wfm(bytes: &[u8], default_channel: &str) -> Result<WaveformTrace, WaveformFileError> {
    if bytes.len() < 830 {
        return Err(WaveformFileError::Parse("WFM file too short".into()));
    }
    if !looks_like_wfm(bytes) {
        // Some scopes mislabel ISF as .wfm
        if looks_like_isf(bytes) {
            return load_tek_isf(bytes, default_channel);
        }
        return Err(WaveformFileError::Parse("not a Tektronix WFM#001 file".into()));
    }

    let le = bytes[0] == 0x0f && bytes[1] == 0x0f;
    let read_u32 = |off: usize| -> u32 {
        if off + 4 > bytes.len() {
            return 0;
        }
        let mut b = [0u8; 4];
        b.copy_from_slice(&bytes[off..off + 4]);
        if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    };
    let read_f64 = |off: usize| -> f64 {
        if off + 8 > bytes.len() {
            return 0.0;
        }
        let mut b = [0u8; 8];
        b.copy_from_slice(&bytes[off..off + 8]);
        if le {
            f64::from_le_bytes(b)
        } else {
            f64::from_be_bytes(b)
        }
    };
    let read_i16 = |off: usize| -> i16 {
        if off + 2 > bytes.len() {
            return 0;
        }
        let mut b = [0u8; 2];
        b.copy_from_slice(&bytes[off..off + 2]);
        if le {
            i16::from_le_bytes(b)
        } else {
            i16::from_be_bytes(b)
        }
    };

    let curve_buf_offset = read_u32(16) as usize;
    let bytes_per_pt = bytes.get(15).copied().unwrap_or(2).max(1) as usize;

    let t_scale = read_f64(478);
    let t_offset = read_f64(486);
    let v_scale = read_f64(174);
    let v_offset = read_f64(182);

    let data_start = read_u32(804) as usize;
    let postcharge_start = read_u32(808) as usize;

    if curve_buf_offset >= bytes.len() {
        return Err(WaveformFileError::Parse("WFM curve buffer offset out of range".into()));
    }

    let base = curve_buf_offset;
    let data_off = base.saturating_add(data_start);
    let data_end = base.saturating_add(postcharge_start.max(data_start + bytes_per_pt));
    if data_end > bytes.len() || data_off >= data_end {
        return Err(WaveformFileError::Parse("WFM curve slice invalid".into()));
    }

    let n_bytes = data_end - data_off;
    let n_pts = n_bytes / bytes_per_pt;
    if n_pts == 0 {
        return Err(WaveformFileError::Parse("WFM produced no samples".into()));
    }

    let mut x = Vec::with_capacity(n_pts);
    let mut y = Vec::with_capacity(n_pts);
    for i in 0..n_pts {
        let off = data_off + i * bytes_per_pt;
        let raw = if bytes_per_pt >= 2 {
            read_i16(off) as f64
        } else {
            bytes[off] as i8 as f64
        };
        x.push(t_offset + i as f64 * t_scale);
        y.push(raw * v_scale + v_offset);
    }

    Ok(WaveformTrace {
        channel: default_channel.to_string(),
        x,
        y,
        x_unit: "s".into(),
        y_unit: "V".into(),
    })
}

/// Parse Tektronix ISF (internal waveform) files.
fn load_tek_isf(bytes: &[u8], default_channel: &str) -> Result<WaveformTrace, WaveformFileError> {
    // Locate CURVE block. ISF is typically ASCII preamble + `:CURVE #N<data>`.
    let curve_pat = b":CURVE";
    let curve_pat2 = b"CURVE ";
    let curve_at = find_bytes_ci(bytes, curve_pat)
        .or_else(|| find_bytes_ci(bytes, curve_pat2))
        .ok_or_else(|| WaveformFileError::Parse("ISF missing CURVE block".into()))?;

    let preamble = String::from_utf8_lossy(&bytes[..curve_at]);
    let kv = parse_wfmp_keys(&preamble);

    let after = &bytes[curve_at..];
    let hash_at = after
        .iter()
        .position(|&b| b == b'#')
        .ok_or_else(|| WaveformFileError::Parse("ISF CURVE missing # length prefix".into()))?;
    let rest = &after[hash_at + 1..];
    if rest.is_empty() {
        return Err(WaveformFileError::Parse("ISF CURVE truncated".into()));
    }
    let ndigits = (rest[0] as char)
        .to_digit(10)
        .ok_or_else(|| WaveformFileError::Parse("ISF CURVE bad length digit".into()))?
        as usize;
    if rest.len() < 1 + ndigits {
        return Err(WaveformFileError::Parse("ISF CURVE length truncated".into()));
    }
    let len_str = std::str::from_utf8(&rest[1..1 + ndigits])
        .map_err(|_| WaveformFileError::Parse("ISF CURVE length not ASCII".into()))?;
    let data_len: usize = len_str
        .parse()
        .map_err(|_| WaveformFileError::Parse(format!("ISF bad data length: {len_str}")))?;
    let data = &rest[1 + ndigits..];
    if data.len() < data_len {
        return Err(WaveformFileError::Parse(format!(
            "ISF data short: need {data_len}, got {}",
            data.len()
        )));
    }
    let data = &data[..data_len];

    let byt_nr = kv_int(&kv, "BYT_NR").unwrap_or(1).max(1) as usize;
    let bn_fmt = kv_str(&kv, "BN_FMT").unwrap_or("RI").to_ascii_uppercase();
    let byt_or = kv_str(&kv, "BYT_OR").unwrap_or("MSB").to_ascii_uppercase();
    let nr_pt = kv_int(&kv, "NR_PT").unwrap_or((data_len / byt_nr) as i64) as usize;
    let xincr = kv_f64(&kv, "XINCR").unwrap_or(1.0);
    let xzero = kv_f64(&kv, "XZERO").unwrap_or(0.0);
    let ymult = kv_f64(&kv, "YMULT").unwrap_or(1.0);
    let yoff = kv_f64(&kv, "YOFF").unwrap_or(0.0);
    let yzero = kv_f64(&kv, "YZERO").unwrap_or(0.0);
    let x_unit = kv_str(&kv, "XUNIT").unwrap_or("s").replace('"', "");
    let y_unit = kv_str(&kv, "YUNIT").unwrap_or("V").replace('"', "");
    let channel = kv_str(&kv, "WFID")
        .or_else(|| kv_str(&kv, "SOURCE"))
        .map(|s| sanitize_channel_name(&s.replace('"', "")))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_channel.to_string());

    let samples = decode_curve_samples(data, byt_nr, &bn_fmt, &byt_or, nr_pt)?;
    let mut x = Vec::with_capacity(samples.len());
    let mut y = Vec::with_capacity(samples.len());
    for (i, raw) in samples.into_iter().enumerate() {
        x.push(xzero + i as f64 * xincr);
        y.push(yzero + ymult * (raw - yoff));
    }

    if x.is_empty() {
        return Err(WaveformFileError::Parse("ISF produced no samples".into()));
    }

    Ok(WaveformTrace {
        channel,
        x,
        y,
        x_unit,
        y_unit,
    })
}

fn decode_curve_samples(
    data: &[u8],
    byt_nr: usize,
    bn_fmt: &str,
    byt_or: &str,
    nr_pt: usize,
) -> Result<Vec<f64>, WaveformFileError> {
    let msb = byt_or.contains("MSB");
    let signed = bn_fmt.contains('R'); // RI = signed, RP = unsigned
    let mut out = Vec::with_capacity(nr_pt.min(data.len() / byt_nr.max(1)));
    let mut i = 0usize;
    while i + byt_nr <= data.len() && out.len() < nr_pt {
        let raw = match byt_nr {
            1 => {
                let v = data[i];
                if signed {
                    v as i8 as f64
                } else {
                    v as f64
                }
            }
            2 => {
                let (lo, hi) = if msb {
                    (data[i + 1], data[i])
                } else {
                    (data[i], data[i + 1])
                };
                let u = u16::from_le_bytes([lo, hi]);
                if signed {
                    u as i16 as f64
                } else {
                    u as f64
                }
            }
            4 => {
                let bytes = if msb {
                    [data[i + 3], data[i + 2], data[i + 1], data[i]]
                } else {
                    [data[i], data[i + 1], data[i + 2], data[i + 3]]
                };
                let u = u32::from_le_bytes(bytes);
                if signed {
                    u as i32 as f64
                } else {
                    u as f64
                }
            }
            _ => {
                return Err(WaveformFileError::Parse(format!(
                    "unsupported ISF BYT_NR={byt_nr}"
                )));
            }
        };
        out.push(raw);
        i += byt_nr;
    }
    Ok(out)
}

fn parse_wfmp_keys(preamble: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // Normalize separators: KEY VALUE; KEY VALUE
    let cleaned = preamble.replace('\r', "\n");
    for chunk in cleaned.split(|c| c == ';' || c == '\n') {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        let chunk = chunk.trim_start_matches(':');
        // Forms: WFMPRE:XINCR 1e-9  OR  XINCR 1e-9  OR  XINCR:1e-9
        let chunk = chunk
            .strip_prefix("WFMPRE:")
            .or_else(|| chunk.strip_prefix("WFMP:"))
            .or_else(|| chunk.strip_prefix("WFMOutpre:"))
            .unwrap_or(chunk);
        let (k, v) = if let Some((k, v)) = chunk.split_once(':') {
            (k.trim(), v.trim())
        } else if let Some((k, v)) = chunk.split_once(char::is_whitespace) {
            (k.trim(), v.trim())
        } else {
            continue;
        };
        if k.is_empty() || v.is_empty() {
            continue;
        }
        // Nested "WFMPRE XINCR ..." already stripped; also handle "XINCR 1.0"
        let key = k
            .rsplit([':', ' '])
            .next()
            .unwrap_or(k)
            .trim()
            .to_ascii_uppercase();
        out.push((key, v.to_string()));
    }
    out
}

fn kv_str<'a>(kv: &'a [(String, String)], key: &str) -> Option<&'a str> {
    kv.iter()
        .rev()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

fn kv_f64(kv: &[(String, String)], key: &str) -> Option<f64> {
    kv_str(kv, key)?.trim_matches('"').parse().ok()
}

fn kv_int(kv: &[(String, String)], key: &str) -> Option<i64> {
    kv_str(kv, key)?.trim_matches('"').parse().ok()
}

fn find_bytes_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let nlen = needle.len();
    'outer: for i in 0..=hay.len() - nlen {
        for j in 0..nlen {
            let a = hay[i + j].to_ascii_uppercase();
            let b = needle[j].to_ascii_uppercase();
            if a != b {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_wiparse_csv_roundtrip() {
        let csv = "channel,index,x(s),y(V)\nCH1,0,0.0,0.0\nCH1,1,1e-6,1.0\nCH1,2,2e-6,-1.0\n";
        let trace = load_waveform_csv_bytes(csv.as_bytes(), "CH1").unwrap();
        assert_eq!(trace.channel, "CH1");
        assert_eq!(trace.x.len(), 3);
        assert_eq!(trace.x_unit, "s");
        assert_eq!(trace.y_unit, "V");
        let m = measure_waveform(&trace);
        assert_eq!(m.count, 3);
        assert!((m.pp - 2.0).abs() < 1e-9);
    }

    #[test]
    fn load_simple_xy_csv() {
        let csv = "TIME,CH1\n0,0\n1e-3,1\n2e-3,0\n3e-3,-1\n4e-3,0\n";
        let trace = load_waveform_csv_bytes(csv.as_bytes(), "CH1").unwrap();
        assert_eq!(trace.x.len(), 5);
        assert!(trace.channel.to_ascii_uppercase().contains("CH"));
    }

    #[test]
    fn load_tek_isf_ri_bytes() {
        // Minimal synthetic ISF: 4 signed bytes, YMULT=1, YOFF=0
        let mut bytes = b":WFMPRE:BYT_NR 1;BIT_NR 8;ENCDG BIN;BN_FMT RI;BYT_OR MSB;NR_PT 4;XINCR 1.0E-6;XZERO 0;XUNIT \"s\";YMULT 1;YOFF 0;YZERO 0;YUNIT \"V\";:CURVE #14".to_vec();
        bytes.extend_from_slice(&[0i8 as u8, 10i8 as u8, 0i8 as u8, (-10i8) as u8]); // 0,10,0,-10
        let trace = load_tek_isf(&bytes, "CH1").unwrap();
        assert_eq!(trace.y.len(), 4);
        assert!((trace.y[1] - 10.0).abs() < 1e-9);
        assert!((trace.y[3] + 10.0).abs() < 1e-9);
        assert!((trace.x[1] - 1e-6).abs() < 1e-15);
    }

    #[test]
    fn wfm_export_load_roundtrip() {
        let trace = WaveformTrace {
            channel: "CH1".into(),
            x: vec![0.0, 1e-6, 2e-6, 3e-6],
            y: vec![0.0, 1.0, -1.0, 0.5],
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let dir = std::env::temp_dir().join(format!("wiparse_wfm_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.wfm");
        export_waveform_wfm(&path, &trace).unwrap();
        let loaded = load_tek_wfm(&std::fs::read(&path).unwrap(), "CH1").unwrap();
        assert_eq!(loaded.y.len(), 4);
        assert!((loaded.y[1] - 1.0).abs() < 0.05);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
