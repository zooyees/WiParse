//! Offline oscilloscope waveform file loaders and measurements.
//!
//! Supported:
//! - WiParse CSV export (`channel,index,x(unit),y(unit)`)
//! - Simple 2-column numeric CSV (`x,y` / `TIME,CH1`)
//! - Tektronix spreadsheet CSV (metadata header + TIME/CHx columns)
//! - Tektronix ISF (WFMPRE ASCII preamble + CURVE binary block)
//! - Tektronix WFM#001 / #002 / #003 (Windows reference waveform, YT)
//! - Rigol DS1000Z / DS1000B / DS4000 / DHO800 proprietary `.wfm`

use crate::instrument::WaveformTrace;
use crate::rigol_wfm::{load_rigol_wfm_all, looks_like_rigol_wfm};
use std::path::Path;
use thiserror::Error;

thread_local! {static POINT_BUDGET:std::cell::Cell<Option<usize>>=const{std::cell::Cell::new(None)};}
pub(crate) fn consume_wave_points(count:usize)->Result<(),WaveformFileError>{POINT_BUDGET.with(|budget|{if let Some(left)=budget.get(){if count>left{return Err(WaveformFileError::Parse("Waveform allocation budget exceeded; increase the memory budget or remove other files".into()))}budget.set(Some(left-count));}Ok(())})}
pub(crate) fn check_wave_points(count:usize)->Result<(),WaveformFileError>{POINT_BUDGET.with(|budget|{if budget.get().is_some_and(|left|count>left){return Err(WaveformFileError::Parse("Waveform allocation budget exceeded".into()))}Ok(())})}
pub fn load_waveform_file_bounded(path:impl AsRef<Path>,points:usize)->Result<Vec<WaveformTrace>,WaveformFileError>{
 struct Reset(Option<usize>);impl Drop for Reset{fn drop(&mut self){POINT_BUDGET.with(|b|b.set(self.0));}}
 let _reset=Reset(POINT_BUDGET.with(|b|b.replace(Some(points))));load_waveform_file_all(path)
}

/// Files larger than this are memory-mapped for parse (avoids an extra RAM copy).
const MMAP_THRESHOLD: u64 = 16 * 1024 * 1024;

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
///
/// For multi-curve Tek WFM (FastFrame / labeled `CH1|CH2|…`), returns the first
/// channel. Use [`load_waveform_file_all`] to get every channel.
pub fn load_waveform_file(path: impl AsRef<Path>) -> Result<WaveformTrace, WaveformFileError> {
    let mut traces = load_waveform_file_all(path)?;
    Ok(traces.remove(0))
}

/// Load all traces from a waveform file (multi-channel / FastFrame WFM expands to N traces).
pub fn load_waveform_file_all(
    path: impl AsRef<Path>,
) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    let path = path.as_ref();
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
    load_waveform_file_at(path, &ext, &stem)
}

fn load_waveform_file_at(
    path: &Path,
    ext: &str,
    stem: &str,
) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    let mut options=std::fs::OpenOptions::new();options.read(true);
    #[cfg(windows)]{use std::os::windows::fs::OpenOptionsExt;options.share_mode(1);}
    let mut file=options.open(path)?;let size=file.metadata()?.len();
    if size>=MMAP_THRESHOLD{let map=unsafe{memmap2::Mmap::map(&file)?};return load_waveform_bytes_all(&map,ext,stem)}
    use std::io::Read;let mut bytes=Vec::new();file.read_to_end(&mut bytes)?;load_waveform_bytes_all(&bytes,ext,stem)
}

/// Load all traces from bytes (see [`load_waveform_file_all`]).
pub fn load_waveform_bytes_all(
    bytes: &[u8],
    hint_ext: &str,
    default_channel: &str,
) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    let ext = hint_ext.trim().trim_start_matches('.').to_ascii_lowercase();
    let traces = match ext.as_str() {
        "isf" => load_tek_isf_all(bytes, default_channel)?,
        "wfm" => load_wfm_bytes_all(bytes, default_channel)?,
        "csv" | "txt" => load_csv_bytes_all(bytes, default_channel)?,
        _ => {
            if looks_like_wfm(bytes) {
                load_tek_wfm_all(bytes, default_channel)?
            } else if looks_like_rigol_wfm(bytes) {
                load_rigol_wfm_all(bytes)?
            } else if looks_like_isf(bytes) {
                load_tek_isf_all(bytes, default_channel)?
            } else {
                load_csv_bytes_all(bytes, default_channel)?
            }
        }
    };
    if traces.is_empty() {
        return Err(WaveformFileError::Parse("empty waveform".into()));
    }
    Ok(traces)
}

/// Load waveform bytes with an optional extension hint (`isf` / `wfm` / `csv`).
pub fn load_waveform_bytes(
    bytes: &[u8],
    hint_ext: &str,
    default_channel: &str,
) -> Result<WaveformTrace, WaveformFileError> {
    let mut traces = load_waveform_bytes_all(bytes, hint_ext, default_channel)?;
    Ok(traces.remove(0))
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
    std::fs::write(path, waveform_to_wiparse_csv(trace))?;
    Ok(())
}

/// WiParse native CSV (`channel,index,x,y`).
pub fn waveform_to_wiparse_csv(trace: &WaveformTrace) -> Vec<u8> {
    use std::fmt::Write as _;
    let mut csv = String::new();
    let _ = writeln!(
        csv,
        "channel,index,x({}),y({})",
        trace.x_unit, trace.y_unit
    );
    let n = trace.x.len().min(trace.y.len());
    for i in 0..n {
        let _ = writeln!(
            csv,
            "{},{},{},{}",
            csv_cell(&trace.channel),
            i,
            trace.x[i],
            trace.y[i]
        );
    }
    csv.into_bytes()
}

/// Stream the same WiParse CSV representation without retaining an additional
/// full-size copy of a large waveform in memory.
pub fn write_waveform_csv(mut writer: impl std::io::Write, trace: &WaveformTrace) -> std::io::Result<()> {
    writeln!(writer, "channel,index,x({}),y({})", trace.x_unit, trace.y_unit)?;
    let channel=csv_cell(&trace.channel);
    for (i,(&x,&y)) in trace.x.iter().zip(trace.y.iter()).enumerate() {
        writeln!(writer,"{channel},{i},{x},{y}")?;
    }
    Ok(())
}

/// Concatenate multiple single-channel Tek ISF blobs into one multi-curve file.
///
/// Each channel keeps its own `:WFMPRE … :CURVE #…` segment. WiParse (and many
/// Tek tools that scan for successive CURVE blocks) can reload every channel.
pub fn join_tek_isf_channels(channel_isfs: &[(u8, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, (ch, bytes)) in channel_isfs.iter().enumerate() {
        if i > 0 {
            out.push(b'\n');
        }
        // Ensure a channel tag exists for reload labeling.
        let tagged = ensure_isf_wfid(bytes, &format!("CH{ch}"));
        out.extend_from_slice(&tagged);
    }
    out
}

fn ensure_isf_wfid(bytes: &[u8], channel: &str) -> Vec<u8> {
    let upper = String::from_utf8_lossy(bytes).to_ascii_uppercase();
    if upper.contains("WFID") || upper.contains("SOURCE") {
        return bytes.to_vec();
    }
    // Insert WFID before :CURVE when missing.
    if let Some(at) = find_bytes_ci(bytes, b":CURVE").or_else(|| find_bytes_ci(bytes, b"CURVE ")) {
        let mut out = Vec::with_capacity(bytes.len() + 32);
        out.extend_from_slice(&bytes[..at]);
        if !out.last().is_some_and(|b| *b == b';' || *b == b'\n') {
            out.push(b';');
        }
        out.extend_from_slice(format!("WFID \"{channel}\";").as_bytes());
        out.extend_from_slice(&bytes[at..]);
        out
    } else {
        bytes.to_vec()
    }
}

/// Spreadsheet CSV (`TIME,CHx`) — Excel / Rigol / Tek friendly.
pub fn waveform_to_spreadsheet_csv(trace: &WaveformTrace) -> Vec<u8> {
    waveforms_to_spreadsheet_csv(std::slice::from_ref(trace))
}

/// Multi-channel spreadsheet CSV (`TIME,CH1,CH2,…`).
///
/// Rows are aligned by sample index using the first trace’s time axis. Channels
/// with fewer points leave trailing cells empty.
pub fn waveforms_to_spreadsheet_csv(traces: &[WaveformTrace]) -> Vec<u8> {
    use std::fmt::Write as _;
    if traces.is_empty() {
        return b"TIME\n".to_vec();
    }
    let mut csv = String::new();
    let names: Vec<String> = traces
        .iter()
        .map(|t| {
            if t.channel.trim().is_empty() {
                "CH?".into()
            } else {
                sanitize_channel_name(&t.channel)
            }
        })
        .collect();
    let _ = write!(csv, "TIME");
    for name in &names {
        let _ = write!(csv, ",{}", csv_cell(name));
    }
    let _ = writeln!(csv);
    let n = traces
        .iter()
        .map(|t| t.x.len().min(t.y.len()))
        .max()
        .unwrap_or(0);
    let time = &traces[0].x;
    for i in 0..n {
        let t = time.get(i).copied().unwrap_or(f64::NAN);
        let _ = write!(csv, "{}", format_csv_f64(t));
        for tr in traces {
            let _ = write!(csv, ",");
            if let Some(&y) = tr.y.get(i) {
                let _ = write!(csv, "{}", format_csv_f64(y));
            }
        }
        let _ = writeln!(csv);
    }
    csv.into_bytes()
}

fn format_csv_f64(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    // Compact but precise enough for scope timebases.
    let s = format!("{v:.12e}");
    s
}

/// Export trace as Tektronix ISF (WFMPRE + CURVE block).
pub fn export_waveform_isf(
    path: impl AsRef<Path>,
    trace: &WaveformTrace,
) -> Result<(), WaveformFileError> {
    std::fs::write(path, waveform_isf_bytes(trace)?)?;
    Ok(())
}

/// The same ISF encoder used by file export, reusable for multi-channel packaging.
pub fn waveform_isf_bytes(trace: &WaveformTrace) -> Result<Vec<u8>, WaveformFileError> {
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
    // Signed RI: y = YZERO + YMULT * (code - YOFF) with YOFF=0.
    let peak = ymin.abs().max(ymax.abs()).max(1e-12);
    let ymult = peak / 127.0;
    let yoff = 0.0;
    let yzero = 0.0;

    let mut curve = Vec::with_capacity(n);
    for &yv in &trace.y[..n] {
        let code = ((yv - yzero) / ymult).round().clamp(-128.0, 127.0) as i8;
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
    out.extend_from_slice(&curve);
    Ok(out)
}

/// Export trace as Tektronix WFM#001 (YT INT16, single frame).
pub fn export_waveform_wfm(
    path: impl AsRef<Path>,
    trace: &WaveformTrace,
) -> Result<(), WaveformFileError> {
    let n = trace.x.len();
    if n == 0 || n != trace.y.len()
        || trace.x.iter().chain(trace.y.iter()).any(|v| !v.is_finite()) {
        return Err(WaveformFileError::Parse("WFM requires equal, nonempty finite X/Y arrays".into()));
    }
    let dt = if n >= 2 { trace.x[1] - trace.x[0] } else { 1.0 };
    let tolerance = (dt.abs() * 1e-7).max(trace.x[0].abs() * f64::EPSILON * 2.0);
    if !dt.is_finite() || dt <= 0.0 || trace.x.windows(2).any(|w| {
        let step = w[1] - w[0];
        step <= 0.0 || !step.is_finite() || (step - dt).abs() > tolerance
    }) {
        return Err(WaveformFileError::Parse("WFM requires a uniform increasing time axis; use CSV".into()));
    }
    let xzero = trace.x[0];
    let (ymin, ymax) = trace.y[..n].iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
        (lo.min(v), hi.max(v))
    });
    let yrange = (ymax - ymin).max(1e-12);
    let v_scale = yrange / 65534.0;
    let v_offset = ymin * 0.5 + ymax * 0.5;
    if !v_scale.is_finite() || v_scale <= 0.0 || !v_offset.is_finite() {
        return Err(WaveformFileError::Parse("WFM amplitude range is not representable".into()));
    }
    let pre = 16usize;
    let post = 16usize;
    let record = n.checked_add(pre + post).ok_or_else(|| WaveformFileError::Parse("WFM too large".into()))?;
    let bytes_per_pt = 2usize;
    let curve_bytes = record.checked_mul(bytes_per_pt).filter(|v| *v < 999_999_000)
        .ok_or_else(|| WaveformFileError::Parse("WFM too large".into()))?;
    let header_len = 820usize;
    let file_len = header_len + curve_bytes + 8;

    let mut buf = vec![0u8; file_len];
    // Intel byte order marker
    buf[0] = 0x0f;
    buf[1] = 0x0f;
    buf[2..10].copy_from_slice(b":WFM#001");
    let eof_count = file_len - 11;
    buf[10] = eof_count.to_string().len() as u8;
    buf[11..15].copy_from_slice(&(eof_count as i32).to_le_bytes());
    buf[15] = bytes_per_pt as u8;
    buf[16..20].copy_from_slice(&(header_len as i32).to_le_bytes());

    write_i32_le(&mut buf, 20, 1);
    write_f64_le(&mut buf, 28, 1.0);
    write_wfm_string(&mut buf[40..72], &trace.channel);
    buf[76..78].copy_from_slice(&88u16.to_le_bytes());
    for off in [82, 110, 114, 118, 134, 138, 142] { write_u32_le(&mut buf, off, 1); }
    write_u32_le(&mut buf, 102, 5);
    write_u32_le(&mut buf, 106, 1);
    write_u32_le(&mut buf, 122, 2); // VECTOR
    write_u32_le(&mut buf, 154, 1); // YT
    // Tek 001-1378-03: explicit scale/offset/size/units start at 166/174/182/186.
    write_f64_le(&mut buf, 166, v_scale);
    write_f64_le(&mut buf, 174, v_offset);
    write_u32_le(&mut buf, 182, 65534);
    write_wfm_string(&mut buf[186..206], &trace.y_unit);
    write_f64_le(&mut buf, 266, 1.0);
    write_wfm_string(&mut buf[274..294], &trace.y_unit);
    write_u32_le(&mut buf, 302, 1);
    write_i32_le(&mut buf, 394, 6); // unused dimension: V1 INVALID_FORMAT
    write_f64_le(&mut buf, 478, dt);
    write_f64_le(&mut buf, 486, xzero);
    write_u32_le(&mut buf, 494, record as u32);
    write_wfm_string(&mut buf[498..518], &trace.x_unit);
    write_f64_le(&mut buf, 554, 1.0);
    write_wfm_string(&mut buf[562..582], &trace.x_unit);
    write_u32_le(&mut buf, 590, 1);
    write_u32_le(&mut buf, 742, 1);
    write_u32_le(&mut buf, 746, 1); // sample sweep, time base
    write_u32_le(&mut buf, 790, 81);
    write_i32_le(&mut buf, 238, 0); // EXPLICIT_INT16

    let data_start = pre * bytes_per_pt;
    let post_start = (pre + n) * bytes_per_pt;
    write_u32_le(&mut buf, 804, data_start as u32);
    write_u32_le(&mut buf, 808, post_start as u32);
    write_u32_le(&mut buf, 812, curve_bytes as u32);
    write_u32_le(&mut buf, 816, curve_bytes as u32);

    for i in 0..record {
        // Repeated endpoints fill interpolation guards, never synthetic zero spikes.
        let yv = trace.y[i.saturating_sub(pre).min(n - 1)];
        let code = ((yv - v_offset) / v_scale).round().clamp(-32767.0, 32767.0) as i16;
        let off = header_len + i * bytes_per_pt;
        buf[off..off + 2].copy_from_slice(&code.to_le_bytes());
    }

    let checksum: u64 = buf[..file_len - 8].iter().map(|v| *v as u64).sum();
    buf[file_len - 8..].copy_from_slice(&checksum.to_le_bytes());
    std::fs::write(path, buf)?;
    Ok(())
}

fn write_wfm_string(target: &mut [u8], value: &str) {
    let mut len = value.len().min(target.len().saturating_sub(1));
    if let Some(nul) = value.find('\0') { len = len.min(nul); }
    while !value.is_char_boundary(len) { len -= 1; }
    target[..len].copy_from_slice(&value.as_bytes()[..len]);
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

/// True for Tektronix Windows reference WFM (`WFM#001` / `#002` / `#003`).
pub fn looks_like_wfm(bytes: &[u8]) -> bool {
    // Endian marker (2 bytes) + optional ':' + "WFM#00x"
    if bytes.len() < 10 {
        return false;
    }
    let body = &bytes[2..];
    body.starts_with(b"WFM#00") || body.starts_with(b":WFM#00")
}

/// Load `.wfm` bytes: Tektronix first, then Rigol proprietary families.
fn load_wfm_bytes_all(
    bytes: &[u8],
    default_channel: &str,
) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    if looks_like_wfm(bytes) {
        return load_tek_wfm_all(bytes, default_channel);
    }
    if looks_like_rigol_wfm(bytes) {
        return load_rigol_wfm_all(bytes);
    }
    // Ambiguous extension: try Tek, then Rigol.
    match load_tek_wfm_all(bytes, default_channel) {
        Ok(t) => Ok(t),
        Err(tek_err) => match load_rigol_wfm_all(bytes) {
            Ok(t) => Ok(t),
            Err(_) => Err(tek_err),
        },
    }
}

/// Best-effort content sniff for instrument capture / Save-As paths.
pub fn sniff_waveform_ext(bytes: &[u8]) -> Option<&'static str> {
    if looks_like_wfm(bytes) || looks_like_rigol_wfm(bytes) {
        Some("wfm")
    } else if looks_like_isf(bytes) {
        Some("isf")
    } else if looks_like_csv(bytes) || looks_like_text_spreadsheet(bytes) {
        Some("csv")
    } else {
        None
    }
}

fn looks_like_text_spreadsheet(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).to_ascii_uppercase();
    head.contains(',')
        && (head.contains("TIME")
            || head.contains("CHANNEL")
            || head.contains("SECOND")
            || head.contains("VOLT"))
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
///
/// Assumes a non-decreasing time axis (normal for scope captures) and measures
/// via index slices — no temporary x/y copies.
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
    let xs = &trace.x[..n];
    let ys = &trace.y[..n];
    let start = xs.partition_point(|&x| x < lo);
    let end = xs.partition_point(|&x| x <= hi);
    if start >= end {
        return measure_samples(&[], &[]);
    }
    measure_samples(&xs[start..end], &ys[start..end])
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

/// High-time ratio using the midpoint threshold. `None` when amplitude is unusable.
pub fn estimate_duty_cycle(y: &[f64]) -> Option<f64> {
    if y.len() < 2 {
        return None;
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &v in y {
        min = min.min(v);
        max = max.max(v);
    }
    let pp = max - min;
    if !pp.is_finite() || pp <= 0.0 {
        return None;
    }
    let mid = min + pp * 0.5;
    let high = y.iter().filter(|&&v| v >= mid).count();
    Some(high as f64 / y.len() as f64)
}

fn is_volt_unit(unit: &str) -> bool {
    let u = unit.trim();
    u.is_empty() || u.eq_ignore_ascii_case("v") || u.eq_ignore_ascii_case("volts")
}

fn normalize_channel_key(channel: &str) -> String {
    let trimmed = channel.trim();
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("CH") {
        return upper;
    }
    if let Ok(n) = trimmed.parse::<u32>() {
        return format!("CH{n}");
    }
    trimmed.to_string()
}

fn good_reading(value: f64, unit: &str) -> Option<serde_json::Value> {
    if !value.is_finite() {
        return None;
    }
    Some(serde_json::json!({ "value": value, "unit": unit, "quality": "good" }))
}

/// Channel metrics actually measured from samples. Missing keys are omitted, never invented.
pub fn host_channel_metrics(trace: &WaveformTrace) -> serde_json::Map<String, serde_json::Value> {
    let m = measure_waveform(trace);
    let mut row = serde_json::Map::new();
    if m.count > 0 && is_volt_unit(&trace.y_unit) {
        if let Some(v) = good_reading(m.min, "V") {
            row.insert("minimum_v".into(), v);
        }
        if let Some(v) = good_reading(m.max, "V") {
            row.insert("maximum_v".into(), v);
        }
        if let Some(v) = good_reading(m.pp, "V") {
            row.insert("peak_to_peak_v".into(), v);
        }
        if let Some(v) = good_reading(m.mean, "V") {
            row.insert("mean_v".into(), v);
        }
    }
    if let Some(f) = m.freq_hz.filter(|v| *v > 0.0).and_then(|v| good_reading(v, "Hz")) {
        row.insert("frequency_hz".into(), f);
    }
    if let Some(p) = m.period.filter(|v| *v > 0.0).and_then(|v| good_reading(v, "s")) {
        row.insert("period_s".into(), p);
    }
    let n = trace.x.len().min(trace.y.len());
    if let Some(duty) = estimate_duty_cycle(&trace.y[..n]).and_then(|v| good_reading(v, "ratio")) {
        row.insert("duty_cycle".into(), duty);
    }
    row
}

/// Per-channel host metrics for FPGA firmware runtime verification.
pub fn host_waveform_metrics(traces: &[WaveformTrace]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for trace in traces {
        let row = host_channel_metrics(trace);
        if !row.is_empty() {
            map.insert(normalize_channel_key(&trace.channel), serde_json::Value::Object(row));
        }
    }
    serde_json::Value::Object(map)
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

fn wave_text_lines(text:&str)->Result<Vec<&str>,WaveformFileError>{
 let mut lines=Vec::new();for line in text.lines().map(str::trim).filter(|line|!line.is_empty()){
  POINT_BUDGET.with(|budget|{if budget.get().is_some_and(|left|lines.len()>left.saturating_mul(2).saturating_add(4096)){return Err(WaveformFileError::Parse("CSV line index exceeds allocation budget".into()))}Ok(())})?;
  lines.try_reserve(1).map_err(|_|WaveformFileError::Parse("Cannot allocate CSV line index".into()))?;lines.push(line);
 }Ok(lines)
}

fn load_waveform_csv_bytes(bytes: &[u8], default_channel: &str) -> Result<WaveformTrace, WaveformFileError> {
    let text = String::from_utf8_lossy(bytes);
    let lines=wave_text_lines(&text)?;
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
        consume_wave_points(1)?;
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
        x: x.into(),
        y: y.into(),
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
        // Tek / WiParse / Rigol CSV headers: TIME, T, X, Time(s), Second, …
        let looks_time = c0 == "time"
            || c0 == "t"
            || c0.starts_with("time(")
            || c0 == "x"
            || c0.starts_with("x(")
            || c0 == "second"
            || c0 == "seconds"
            || c0.starts_with("second(");
        if !looks_time {
            continue;
        }
        let mut found_y = None;
        for (ci, c) in cols.iter().enumerate().skip(1) {
            let u = c.to_ascii_uppercase();
            // CH1 / CH1V / CHAN1 / VOLT / Voltage / Y
            if u.starts_with("CH")
                || u.starts_with("CHAN")
                || u.contains("VOLT")
                || u == "Y"
                || u.starts_with("Y(")
            {
                found_y = Some(ci);
                channel = sanitize_channel_name(c);
                if let Some(unit) = unit_in_parens(c) {
                    y_unit = unit;
                } else if u.contains("MV") {
                    y_unit = "mV".into();
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
        consume_wave_points(1)?;
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
        x: x.into(),
        y: y.into(),
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
        consume_wave_points(1)?;
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
        x: x.into(),
        y: y.into(),
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

/// Normalize Tek/Rigol unit tokens so current-probe `"A"` / `Amps` map to `A`.
fn normalize_isf_unit(raw: &str, default: &str) -> String {
    let s = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches('\0')
        .trim();
    if s.is_empty() {
        return default.to_string();
    }
    match s.to_ascii_lowercase().as_str() {
        "a" | "aa" | "amp" | "amps" | "ampere" | "amperes" => "A".into(),
        "v" | "volt" | "volts" => "V".into(),
        "w" | "watt" | "watts" => "W".into(),
        "s" | "sec" | "second" | "seconds" => "s".into(),
        _ => s.to_string(),
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

/// Parse Tektronix WFM#001 / #002 / #003 (Windows reference waveform).
fn load_tek_wfm(bytes: &[u8], default_channel: &str) -> Result<WaveformTrace, WaveformFileError> {
    let mut traces = load_tek_wfm_all(bytes, default_channel)?;
    Ok(traces.remove(0))
}

fn load_tek_wfm_all(
    bytes: &[u8],
    default_channel: &str,
) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    if bytes.len() < 40 {
        return Err(WaveformFileError::Parse("WFM file too short".into()));
    }
    if !looks_like_wfm(bytes) {
        if looks_like_isf(bytes) {
            return Ok(vec![load_tek_isf(bytes, default_channel)?]);
        }
        return Err(WaveformFileError::Parse("not a Tektronix WFM file".into()));
    }

    // Real Tek files use `:WFM#00x`. Historical WiParse exports use `WFM#001\0`
    // with fixed offsets. A malformed standard header must not be reinterpreted
    // as that different dialect (its scale/offset fields have different meanings).
    let tek_colon_tag = bytes.get(2) == Some(&b':');
    if tek_colon_tag {
        load_tek_wfm_structured_all(bytes, default_channel)
    } else {
        load_tek_wfm_legacy_fixed(bytes, default_channel)
            .map(|t| vec![t])
            .or_else(|legacy_err| {
                load_tek_wfm_structured_all(bytes, default_channel).map_err(|_| legacy_err)
            })
    }
}

/// Byte-order marker: `0F 0F` → little-endian numerics; `F0 F0` → big-endian.
/// (Matches Tek `tm_data_types` / scope exports; the product names are inverted from the tags.)
fn wfm_is_little_endian(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x0f && bytes[1] == 0x0f
}

fn wfm_version_number(bytes: &[u8]) -> Option<u8> {
    if bytes.len() < 10 {
        return None;
    }
    let v = &bytes[2..10];
    if v.starts_with(b":WFM#003") || v.starts_with(b"WFM#003") {
        Some(3)
    } else if v.starts_with(b":WFM#002") || v.starts_with(b"WFM#002") {
        Some(2)
    } else if v.starts_with(b":WFM#001") || v.starts_with(b"WFM#001") {
        Some(1)
    } else {
        None
    }
}

struct WfmReader<'a> {
    bytes: &'a [u8],
    off: usize,
    le: bool,
}

impl<'a> WfmReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            off: 0,
            le: wfm_is_little_endian(bytes),
        }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.off)
    }

    fn skip(&mut self, n: usize) -> Result<(), WaveformFileError> {
        if self.remaining() < n {
            return Err(WaveformFileError::Parse("WFM truncated while skipping".into()));
        }
        self.off += n;
        Ok(())
    }

    fn read_exact(&mut self, n: usize) -> Result<&'a [u8], WaveformFileError> {
        if self.remaining() < n {
            return Err(WaveformFileError::Parse("WFM truncated".into()));
        }
        let s = &self.bytes[self.off..self.off + n];
        self.off += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, WaveformFileError> {
        Ok(self.read_exact(1)?[0])
    }

    fn i16(&mut self) -> Result<i16, WaveformFileError> {
        let b = self.read_exact(2)?;
        Ok(if self.le {
            i16::from_le_bytes([b[0], b[1]])
        } else {
            i16::from_be_bytes([b[0], b[1]])
        })
    }

    fn u16(&mut self) -> Result<u16, WaveformFileError> {
        let b = self.read_exact(2)?;
        Ok(if self.le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }

    fn i32(&mut self) -> Result<i32, WaveformFileError> {
        let b = self.read_exact(4)?;
        let a = [b[0], b[1], b[2], b[3]];
        Ok(if self.le {
            i32::from_le_bytes(a)
        } else {
            i32::from_be_bytes(a)
        })
    }

    fn u32(&mut self) -> Result<u32, WaveformFileError> {
        Ok(self.i32()? as u32)
    }

    fn f64(&mut self) -> Result<f64, WaveformFileError> {
        let b = self.read_exact(8)?;
        let a = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
        Ok(if self.le {
            f64::from_le_bytes(a)
        } else {
            f64::from_be_bytes(a)
        })
    }

    fn cstr(&mut self, n: usize) -> Result<String, WaveformFileError> {
        let s = self.read_exact(n)?;
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        Ok(String::from_utf8_lossy(&s[..end]).trim().to_string())
    }
}

/// Structured WFM layout (Tek `WfmFormat.unpack_wfm_file`).
/// FastFrame / multi-curve files expand to one [`WaveformTrace`] per curve.
fn load_tek_wfm_structured_all(
    bytes: &[u8],
    default_channel: &str,
) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    let version = wfm_version_number(bytes)
        .ok_or_else(|| WaveformFileError::Parse("unsupported WFM version tag".into()))?;

    let mut r = WfmReader::new(bytes);
    r.skip(2)?; // endian marker
    r.skip(8)?; // :WFM#00x / WFM#00x\0

    // WaveformStaticFileInfo
    let _digits = r.u8()?;
    let _bytes_till_eof = r.u32()?;
    let bytes_per_pt = r.u8()?.max(1) as usize;
    let curve_buf_offset = r.i32()? as usize;
    r.skip(4)?; // horizontal_zoom_scale_factor (i32)
    r.skip(4)?; // horizontal_zoom_position (f32)
    r.skip(8)?; // vertical_zoom_scale_factor (f64)
    r.skip(4)?; // vertical_zoom_position (f32)
    let label = r.cstr(32)?;
    let _nframes = r.u32()?;
    let _header_size = r.u16()?;

    // WaveformHeader
    r.skip(4 + 4 + 8 + 8 + 4 + 4)?; // type..is_static
    let _update_spec_cnt = r.u32()?;
    r.skip(4 + 4 + 4 + 8 + 4 + 4 + 4)?; // dim refs..curve_ref
    let _nreq_ff = r.u32()?;
    let nacq_ff = r.u32()? as usize;

    if version != 1 {
        let _summary_frame_type = r.u16()?;
    }
    // PixMap
    r.skip(4 + 8)?;

    // Explicit dimension #1 (Y) + user view + explicit #2 + user view
    let (v_scale, v_offset, y_unit, curve_fmt) = {
        let scale = r.f64()?;
        let offset = r.f64()?;
        let _size = r.u32()?;
        let units = r.cstr(20)?;
        r.skip(8 * 4)?; // extent/resolution/reference
        let format = r.i32()?;
        let _storage = r.i32()?;
        r.skip(4 * 5)?; // null/over/under/high/low
        skip_wfm_user_view(&mut r, version)?;
        skip_explicit_dimension(&mut r)?;
        skip_wfm_user_view(&mut r, version)?;
        (scale, offset, units, format)
    };

    // Implicit dimension #1 (X) + user view + implicit #2 + user view
    let (t_scale, t_offset, x_unit, n_record) = {
        let scale = r.f64()?;
        let offset = r.f64()?;
        let size = r.u32()? as usize;
        let units = r.cstr(20)?;
        r.skip(8 * 4 + 4)?; // extent/resolution/reference + spacing
        skip_wfm_user_view(&mut r, version)?;
        skip_implicit_dimension(&mut r)?;
        skip_wfm_user_view(&mut r, version)?;
        (scale, offset, units, size)
    };

    // TimeBaseInformation × 2
    r.skip(12 * 2)?;
    // UpdateSpecifications (primary)
    r.skip(24)?;

    // Every extra frame needs a 24-byte update spec and a 30-byte curve spec.
    // Validate against actual bytes and the curve-data offset BEFORE reserving
    // from the untrusted u32 frame count; even a tiny file can declare u32::MAX.
    let headers_end = nacq_ff.checked_mul(54).and_then(|n|n.checked_add(30))
        .and_then(|n|r.off.checked_add(n))
        .ok_or_else(||WaveformFileError::Parse("WFM FastFrame header size overflow".into()))?;
    if curve_buf_offset >= bytes.len() || headers_end > bytes.len() || headers_end > curve_buf_offset {
        return Err(WaveformFileError::Parse("WFM FastFrame headers exceed curve offset or file length".into()));
    }
    let mut curve_ranges: Vec<(usize, usize)> = Vec::with_capacity(1 + nacq_ff);
    // CurveInformation (primary)
    {
        let _state_flags = r.u32()?;
        let _checksum_type = r.i32()?;
        let _checksum = r.i16()?;
        let _precharge_start = r.u32()? as usize;
        let data_start = r.u32()? as usize;
        let postcharge_start = r.u32()? as usize;
        let _postcharge_stop = r.u32()? as usize;
        let _eoc = r.u32()?;
        curve_ranges.push((data_start, postcharge_start));
    }

    // FastFrame update specs then curve specs
    for _ in 0..nacq_ff {
        r.skip(24)?;
    }
    for _ in 0..nacq_ff {
        let _state_flags = r.u32()?;
        let _checksum_type = r.i32()?;
        let _checksum = r.i16()?;
        let _precharge_start = r.u32()? as usize;
        let data_start = r.u32()? as usize;
        let postcharge_start = r.u32()? as usize;
        let _postcharge_stop = r.u32()? as usize;
        let _eoc = r.u32()?;
        curve_ranges.push((data_start, postcharge_start));
    }

    if curve_buf_offset >= bytes.len() {
        return Err(WaveformFileError::Parse("WFM curve buffer offset out of range".into()));
    }
    if !(t_scale.is_finite()
        && t_scale.abs() > 0.0
        && v_scale.is_finite()
        && v_scale.abs() > 0.0
        && t_offset.is_finite()
        && v_offset.is_finite())
    {
        return Err(WaveformFileError::Parse("WFM scales invalid".into()));
    }

    let sample_size = wfm_curve_sample_bytes(curve_fmt, bytes_per_pt)?;
    let channel_names = wfm_channel_names(&label, default_channel, curve_ranges.len());
    let x_unit = normalize_isf_unit(&x_unit, "s");
    let y_unit = normalize_isf_unit(&y_unit, "V");

    let mut traces = Vec::with_capacity(curve_ranges.len());
    for (idx, (data_start, postcharge_start)) in curve_ranges.into_iter().enumerate() {
        let data_off = curve_buf_offset.saturating_add(data_start);
        let data_end =
            curve_buf_offset.saturating_add(postcharge_start.max(data_start + sample_size));
        if data_end > bytes.len() || data_off >= data_end {
            return Err(WaveformFileError::Parse("WFM curve slice invalid".into()));
        }
        let n_from_offsets = (data_end - data_off) / sample_size;
        let n_pts = if n_record > 0 {
            n_record.min(n_from_offsets)
        } else {
            n_from_offsets
        };
        if n_pts == 0 {
            return Err(WaveformFileError::Parse("WFM produced no samples".into()));
        }
        consume_wave_points(n_pts)?;
        let mut x = Vec::with_capacity(n_pts);
        let mut y = Vec::with_capacity(n_pts);
        for i in 0..n_pts {
            let raw = read_wfm_sample(bytes, data_off + i * sample_size, curve_fmt, r.le)?;
            x.push(t_offset + i as f64 * t_scale);
            y.push(raw * v_scale + v_offset);
        }
        traces.push(WaveformTrace {
            channel: channel_names
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("CH{}", idx + 1)),
            x: x.into(),
            y: y.into(),
            x_unit: x_unit.clone(),
            y_unit: y_unit.clone(),
        });
    }
    Ok(traces)
}

fn wfm_channel_names(label: &str, default_channel: &str, n: usize) -> Vec<String> {
    let parts: Vec<String> = label
        .split(|c| c == '|' || c == ',' || c == ';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    if parts.len() >= n {
        return parts.into_iter().take(n).collect();
    }
    if n == 1 {
        return vec![if label.is_empty() {
            default_channel.to_string()
        } else {
            label.to_string()
        }];
    }
    (0..n).map(|i| format!("CH{}", i + 1)).collect()
}

fn skip_wfm_user_view(r: &mut WfmReader<'_>, version: u8) -> Result<(), WaveformFileError> {
    // Ver3: point_density is Double; Ver1/2: UnsignedLong
    r.skip(8)?; // scale
    r.skip(20)?; // units
    r.skip(8)?; // offset
    if version >= 3 {
        r.skip(8)?; // point_density f64
    } else {
        r.skip(4)?; // point_density u32
    }
    r.skip(8 + 8)?; // horizontal_reference + trigger_delay
    Ok(())
}

fn skip_explicit_dimension(r: &mut WfmReader<'_>) -> Result<(), WaveformFileError> {
    r.skip(8 + 8 + 4 + 20 + 8 * 4 + 4 * 7)?;
    Ok(())
}

fn skip_implicit_dimension(r: &mut WfmReader<'_>) -> Result<(), WaveformFileError> {
    r.skip(8 + 8 + 4 + 20 + 8 * 4 + 4)?;
    Ok(())
}

fn wfm_curve_sample_bytes(curve_fmt: i32, bytes_per_pt: usize) -> Result<usize, WaveformFileError> {
    let from_fmt = match curve_fmt {
        0 => 2, // INT16
        1 => 4, // INT32
        2 => 4, // UINT32
        3 => 8, // UINT64
        4 => 4, // FP32
        5 => 8, // FP64
        6 => 1, // UINT8 (v3)
        7 => 1, // INT8 (v3)
        _ => 0,
    };
    let n = if from_fmt > 0 {
        from_fmt
    } else {
        bytes_per_pt
    };
    if n == 0 {
        return Err(WaveformFileError::Parse("WFM bytes-per-point is zero".into()));
    }
    Ok(n)
}

fn read_wfm_sample(
    bytes: &[u8],
    off: usize,
    curve_fmt: i32,
    le: bool,
) -> Result<f64, WaveformFileError> {
    let need = wfm_curve_sample_bytes(curve_fmt, 2)?;
    if off + need > bytes.len() {
        return Err(WaveformFileError::Parse("WFM sample out of range".into()));
    }
    let s = &bytes[off..off + need];
    Ok(match curve_fmt {
        0 => {
            let v = if le {
                i16::from_le_bytes([s[0], s[1]])
            } else {
                i16::from_be_bytes([s[0], s[1]])
            };
            v as f64
        }
        1 => {
            let a = [s[0], s[1], s[2], s[3]];
            (if le {
                i32::from_le_bytes(a)
            } else {
                i32::from_be_bytes(a)
            }) as f64
        }
        2 => {
            let a = [s[0], s[1], s[2], s[3]];
            (if le {
                u32::from_le_bytes(a)
            } else {
                u32::from_be_bytes(a)
            }) as f64
        }
        3 => {
            let a = [s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]];
            (if le {
                u64::from_le_bytes(a)
            } else {
                u64::from_be_bytes(a)
            }) as f64
        }
        4 => {
            let a = [s[0], s[1], s[2], s[3]];
            if le {
                f32::from_le_bytes(a) as f64
            } else {
                f32::from_be_bytes(a) as f64
            }
        }
        5 => {
            let a = [s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]];
            if le {
                f64::from_le_bytes(a)
            } else {
                f64::from_be_bytes(a)
            }
        }
        6 => s[0] as f64,
        7 => s[0] as i8 as f64,
        _ => {
            // Fallback: treat as signed int using declared width.
            match need {
                1 => s[0] as i8 as f64,
                2 => {
                    let v = if le {
                        i16::from_le_bytes([s[0], s[1]])
                    } else {
                        i16::from_be_bytes([s[0], s[1]])
                    };
                    v as f64
                }
                4 => {
                    let a = [s[0], s[1], s[2], s[3]];
                    (if le {
                        i32::from_le_bytes(a)
                    } else {
                        i32::from_be_bytes(a)
                    }) as f64
                }
                _ => 0.0,
            }
        }
    })
}

/// Legacy fixed-offset WFM#001 reader (WiParse export / older docs).
fn load_tek_wfm_legacy_fixed(
    bytes: &[u8],
    default_channel: &str,
) -> Result<WaveformTrace, WaveformFileError> {
    if bytes.len() < 830 {
        return Err(WaveformFileError::Parse("WFM file too short".into()));
    }

    let le = wfm_is_little_endian(bytes);
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

    if !t_scale.is_finite() || t_scale.abs() <= 0.0 || !v_scale.is_finite() || v_scale == 0.0 {
        return Err(WaveformFileError::Parse("legacy WFM scales invalid".into()));
    }
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

    consume_wave_points(n_pts)?;

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
        x: x.into(),
        y: y.into(),
        x_unit: "s".into(),
        y_unit: "V".into(),
    })
}

/// Load every `:WFMPRE … :CURVE` segment from a (possibly multi-channel) ISF.
fn load_tek_isf_all(
    bytes: &[u8],
    default_channel: &str,
) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    let mut traces = Vec::new();
    let mut offset = 0usize;
    let mut idx = 0usize;
    while offset < bytes.len() {
        let slice = &bytes[offset..];
        // Skip leading whitespace / NULs between segments.
        let skip = slice
            .iter()
            .position(|&b| !b.is_ascii_whitespace() && b != 0)
            .unwrap_or(slice.len());
        offset += skip;
        if offset >= bytes.len() {
            break;
        }
        let slice = &bytes[offset..];
        let Some((trace, consumed)) = try_load_tek_isf_segment(slice, default_channel, idx)?
        else {
            break;
        };
        traces.push(trace);
        offset += consumed;
        idx += 1;
    }
    if traces.is_empty() {
        // Fall back to legacy single-curve parse for odd layouts.
        traces.push(load_tek_isf(bytes, default_channel)?);
    }
    Ok(traces)
}

fn try_load_tek_isf_segment(
    bytes: &[u8],
    default_channel: &str,
    index: usize,
) -> Result<Option<(WaveformTrace, usize)>, WaveformFileError> {
    let curve_pat = b":CURVE";
    let curve_pat2 = b"CURVE ";
    let Some(curve_at) = find_bytes_ci(bytes, curve_pat).or_else(|| find_bytes_ci(bytes, curve_pat2))
    else {
        return Ok(None);
    };
    let after = &bytes[curve_at..];
    let Some(hash_at) = after.iter().position(|&b| b == b'#') else {
        return Ok(None);
    };
    let rest = &after[hash_at + 1..];
    if rest.is_empty() {
        return Ok(None);
    }
    let Some(ndigits) = (rest[0] as char).to_digit(10).map(|d| d as usize) else {
        return Ok(None);
    };
    if rest.len() < 1 + ndigits {
        return Ok(None);
    }
    let Ok(len_str) = std::str::from_utf8(&rest[1..1 + ndigits]) else {
        return Ok(None);
    };
    let Ok(data_len) = len_str.parse::<usize>() else {
        return Ok(None);
    };
    let data_start = curve_at + hash_at + 1 + 1 + ndigits;
    let data_end = data_start + data_len;
    if data_end > bytes.len() {
        return Err(WaveformFileError::Parse(format!(
            "ISF data short: need {data_len}, got {}",
            bytes.len().saturating_sub(data_start)
        )));
    }
    let fallback = if index == 0 {
        default_channel.to_string()
    } else {
        format!("CH{}", index + 1)
    };
    let trace = load_tek_isf(&bytes[..data_end], &fallback)?;
    Ok(Some((trace, data_end)))
}

fn load_csv_bytes_all(
    bytes: &[u8],
    default_channel: &str,
) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    let text = String::from_utf8_lossy(bytes);
    let lines=wave_text_lines(&text)?;
    if let Some(traces) = try_parse_spreadsheet_csv_multi(&lines)? {
        return Ok(traces);
    }
    Ok(vec![load_waveform_csv_bytes(bytes, default_channel)?])
}

/// `TIME,CH1,CH2,…` → one trace per Y column.
fn try_parse_spreadsheet_csv_multi(
    lines: &[&str],
) -> Result<Option<Vec<WaveformTrace>>, WaveformFileError> {
    if lines.is_empty() {
        return Ok(None);
    }
    let header = split_csv(lines[0]);
    if header.len() < 3 {
        return Ok(None);
    }
    let h0 = header[0].to_ascii_uppercase();
    if !(h0.contains("TIME") || h0 == "T" || h0 == "X" || h0.contains("SECOND")) {
        return Ok(None);
    }
    let names: Vec<String> = header[1..]
        .iter()
        .map(|s| sanitize_channel_name(s))
        .filter(|s| !s.is_empty())
        .collect();
    if names.len() < 2 {
        return Ok(None);
    }
    let mut xs = Vec::new();
    let mut ys: Vec<Vec<f64>> = names.iter().map(|_| Vec::new()).collect();
    for line in &lines[1..] {
        let cols = split_csv(line);
        if cols.len() < 2 {
            continue;
        }
        let Ok(t) = cols[0].parse::<f64>() else {
            continue;
        };
        consume_wave_points(ys.len())?;
        xs.push(t);
        for (i, col) in ys.iter_mut().enumerate() {
            let v = cols
                .get(i + 1)
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(f64::NAN);
            col.push(v);
        }
    }
    if xs.len() < 2 {
        return Ok(None);
    }
    let x_shared: std::sync::Arc<[f64]> = xs.into();
    let traces = names
        .into_iter()
        .zip(ys)
        .map(|(channel, y)| WaveformTrace {
            channel,
            x: std::sync::Arc::clone(&x_shared),
            y: y.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        })
        .collect();
    Ok(Some(traces))
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
    // Trigger sample index — match VISA CURVe path: x = XZERO + (i - PT_OFF)*XINCR
    let pt_off = kv_f64(&kv, "PT_OFF")
        .or_else(|| kv_f64(&kv, "PT_OFf"))
        .unwrap_or(0.0);
    let ymult = kv_f64(&kv, "YMULT").unwrap_or(1.0);
    let yoff = kv_f64(&kv, "YOFF").unwrap_or(0.0);
    let yzero = kv_f64(&kv, "YZERO").unwrap_or(0.0);
    let pt_fmt = kv_str(&kv, "PT_FMT").unwrap_or("Y").to_ascii_uppercase();
    let x_unit = kv_str(&kv, "XUNIT").unwrap_or("s").replace('"', "");
    let y_unit = normalize_isf_unit(
        &kv_str(&kv, "YUNIT")
            .or_else(|| kv_str(&kv, "YUNit"))
            .unwrap_or("V")
            .replace('"', ""),
        "V",
    );
    let channel = kv_str(&kv, "WFID")
        .or_else(|| kv_str(&kv, "SOURCE"))
        .map(|s| sanitize_channel_name(&s.replace('"', "")))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_channel.to_string());

    let mut samples = decode_curve_samples(data, byt_nr, &bn_fmt, &byt_or, nr_pt)?;
    // ENV = min/max pairs; convert to midpoints so amplitude matches the screen.
    if pt_fmt.contains("ENV") && samples.len() >= 2 {
        let mut mid = Vec::with_capacity(samples.len() / 2);
        for pair in samples.chunks_exact(2) {
            mid.push(0.5 * (pair[0] + pair[1]));
        }
        if !mid.is_empty() {
            samples = mid;
        }
    }
    consume_wave_points(samples.len())?;
    let mut x = Vec::with_capacity(samples.len());
    let mut y = Vec::with_capacity(samples.len());
    for (i, raw) in samples.into_iter().enumerate() {
        x.push(xzero + (i as f64 - pt_off) * xincr);
        y.push(yzero + ymult * (raw - yoff));
    }

    if x.is_empty() {
        return Err(WaveformFileError::Parse("ISF produced no samples".into()));
    }

    Ok(WaveformTrace {
        channel,
        x: x.into(),
        y: y.into(),
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
    check_wave_points(nr_pt.min(data.len()/byt_nr.max(1)))?;
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
        // Prefix strip must be case-insensitive (Tek may return WFMOutpre / WFMOUTPRE).
        let upper = chunk.to_ascii_uppercase();
        let chunk = if let Some(rest) = upper.strip_prefix("WFMOUTPRE:") {
            // Keep values from the original slice at the same offset.
            &chunk[chunk.len() - rest.len()..]
        } else if let Some(rest) = upper.strip_prefix("WFMPRE:") {
            &chunk[chunk.len() - rest.len()..]
        } else if let Some(rest) = upper.strip_prefix("WFMP:") {
            &chunk[chunk.len() - rest.len()..]
        } else {
            chunk
        };
        let (k, v) = if let Some((k, v)) = chunk.split_once(':') {
            // Avoid splitting scientific values; only treat KEY:VALUE when KEY is a token.
            if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                (k.trim(), v.trim())
            } else if let Some((k2, v2)) = chunk.split_once(char::is_whitespace) {
                (k2.trim(), v2.trim())
            } else {
                continue;
            }
        } else if let Some((k, v)) = chunk.split_once(char::is_whitespace) {
            (k.trim(), v.trim())
        } else {
            continue;
        };
        if k.is_empty() || v.is_empty() {
            continue;
        }
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
    fn streamed_csv_matches_existing_format_and_preserves_samples() {
        let trace=WaveformTrace{channel:"CH,1".into(),x:vec![0.0,0.5,1.0].into(),y:vec![-1.0,0.0,1.0].into(),x_unit:"s".into(),y_unit:"V".into()};
        let mut streamed=Vec::new();write_waveform_csv(&mut streamed,&trace).unwrap();
        assert_eq!(streamed,waveform_to_wiparse_csv(&trace));
        let parsed=load_waveform_bytes(&streamed,"csv","CH1").unwrap();
        assert_eq!(parsed.channel,trace.channel);assert_eq!(parsed.x.as_ref(),trace.x.as_ref());assert_eq!(parsed.y.as_ref(),trace.y.as_ref());
    }

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
    fn join_and_load_multi_channel_isf() {
        let mut ch1 = b":WFMPRE:BYT_NR 1;BN_FMT RI;BYT_OR MSB;NR_PT 2;XINCR 1e-6;XZERO 0;YMULT 1;YOFF 0;YZERO 0;WFID \"CH1\";:CURVE #12".to_vec();
        ch1.extend_from_slice(&[0u8, 10u8]);
        let mut ch2 = b":WFMPRE:BYT_NR 1;BN_FMT RI;BYT_OR MSB;NR_PT 2;XINCR 1e-6;XZERO 0;YMULT 1;YOFF 0;YZERO 0;WFID \"CH2\";:CURVE #12".to_vec();
        ch2.extend_from_slice(&[5u8, 15u8]);
        let joined = join_tek_isf_channels(&[(1, ch1.as_slice()), (2, ch2.as_slice())]);
        let traces = load_tek_isf_all(&joined, "CH1").unwrap();
        assert_eq!(traces.len(), 2);
        assert!(traces[0].channel.to_ascii_uppercase().contains("CH1"));
        assert!(traces[1].channel.to_ascii_uppercase().contains("CH2"));
        assert_eq!(traces[0].y.len(), 2);
        assert_eq!(traces[1].y.len(), 2);
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
    fn load_tek_isf_current_probe_yunit_a() {
        // Current probe: YUNIT "A", YMULT in A/div-code (values are amperes).
        let mut bytes = b":WFMPRE:BYT_NR 1;BN_FMT RI;BYT_OR MSB;NR_PT 4;XINCR 1.0E-6;XZERO 0;XUNIT \"s\";YMULT 0.01;YOFF 0;YZERO 0;YUNIT \"A\";WFID \"CH1\";:CURVE #14".to_vec();
        bytes.extend_from_slice(&[0i8 as u8, 100i8 as u8, 0i8 as u8, (-50i8) as u8]);
        let trace = load_tek_isf(&bytes, "CH1").unwrap();
        assert_eq!(trace.y_unit, "A");
        assert!((trace.y[1] - 1.0).abs() < 1e-9); // 100 * 0.01 A
        assert!((trace.y[3] + 0.5).abs() < 1e-9);
    }

    #[test]
    fn load_csv_current_channel_unit() {
        let csv = "Time(s),CH1(A)\n0.0,0.1\n1e-6,0.2\n2e-6,-0.1\n";
        let trace = load_waveform_csv_bytes(csv.as_bytes(), "CH1").unwrap();
        assert_eq!(trace.y_unit, "A");
        assert!((trace.y[1] - 0.2).abs() < 1e-12);
    }

    #[test]
    fn load_tek_isf_applies_pt_off() {
        let mut bytes = b":WFMPRE:BYT_NR 1;BN_FMT RI;BYT_OR MSB;NR_PT 3;XINCR 1.0E-6;XZERO 0;PT_OFF 1;YMULT 1;YOFF 0;YZERO 0;:CURVE #13".to_vec();
        bytes.extend_from_slice(&[1u8, 2u8, 3u8]);
        let trace = load_tek_isf(&bytes, "CH1").unwrap();
        // i=0 → (0-1)*1e-6 = -1e-6; i=1 → 0; i=2 → 1e-6
        assert!((trace.x[0] + 1e-6).abs() < 1e-15);
        assert!(trace.x[1].abs() < 1e-15);
        assert!((trace.x[2] - 1e-6).abs() < 1e-15);
    }

    #[test]
    fn load_rigol_style_csv() {
        let csv = "Time(s),CH1(V)\n0.0,0.1\n1e-6,0.2\n2e-6,-0.1\n";
        let trace = load_waveform_csv_bytes(csv.as_bytes(), "CH1").unwrap();
        assert_eq!(trace.x.len(), 3);
        assert_eq!(trace.x_unit, "s");
        assert_eq!(trace.y_unit, "V");
        assert!((trace.y[1] - 0.2).abs() < 1e-12);
    }

    #[test]
    fn spreadsheet_csv_roundtrip() {
        let trace = WaveformTrace {
            channel: "CH2".into(),
            x: vec![0.0, 1e-6].into(),
            y: vec![0.5, -0.5].into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let bytes = waveform_to_spreadsheet_csv(&trace);
        let loaded = load_waveform_csv_bytes(&bytes, "CH1").unwrap();
        assert_eq!(loaded.x.len(), 2);
        assert!(loaded.channel.to_ascii_uppercase().contains("CH2"));
    }

    #[test]
    fn wfm_export_load_roundtrip() {
        let trace = WaveformTrace {
            channel: "CH1".into(),
            x: vec![0.0, 1e-6, 2e-6, 3e-6].into(),
            y: vec![0.0, 1.0, -1.0, 0.5].into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let dir = std::env::temp_dir().join(format!("wiparse_wfm_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.wfm");
        export_waveform_wfm(&path, &trace).unwrap();
        let loaded = load_tek_wfm(&std::fs::read(&path).unwrap(), "CH1").unwrap();
        assert_eq!(loaded.y.len(), 4);
        for (actual, expected) in loaded.y.iter().zip(trace.y.iter()) {
            assert!((actual - expected).abs() <= 1.0 / 65534.0 + 1e-12);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wfm_standard_header_preserves_bias_constants_and_units() {
        let dir = std::env::temp_dir().join(format!("wiparse_wfm_standard_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (case, bias, amplitude) in [("bipolar", 0.0, 3.3), ("positive", 5.0, 1.0),
            ("negative", -5.0, 1.0), ("constant", 2.5, 0.0)] {
            let trace = WaveformTrace {
                channel: "CH2".into(),
                x: (0..128).map(|i| -0.0005 + i as f64 * 1e-6).collect::<Vec<_>>().into(),
                y: (0..128).map(|i| bias + amplitude * (i as f64 * 0.1).sin()).collect::<Vec<_>>().into(),
                x_unit: "s".into(), y_unit: "A".into(),
            };
            let path = dir.join(format!("{case}.wfm"));
            export_waveform_wfm(&path, &trace).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(&bytes[..10], b"\x0f\x0f:WFM#001");
            assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 820);
            assert_eq!(&bytes[186..188], b"A\0");
            assert_eq!(&bytes[498..500], b"s\0");
            let scale = f64::from_le_bytes(bytes[166..174].try_into().unwrap());
            let sum: u64 = bytes[..bytes.len()-8].iter().map(|b| *b as u64).sum();
            assert_eq!(u64::from_le_bytes(bytes[bytes.len()-8..].try_into().unwrap()), sum);
            // Use only the structured reader: legacy fallback cannot hide a malformed header.
            let loaded = load_tek_wfm_structured_all(&bytes, "fallback").unwrap().remove(0);
            assert_eq!(loaded.channel, "CH2");
            assert_eq!(loaded.y_unit, "A");
            assert_eq!(loaded.y.len(), trace.y.len());
            for i in 0..128 {
                assert!((loaded.y[i] - trace.y[i]).abs() <= scale * 0.5 + 1e-12, "{case} sample {i}");
                assert!((loaded.x[i] - trace.x[i]).abs() < 1e-15);
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wfm_export_rejects_invalid_arrays_without_creating_output() {
        let path = std::env::temp_dir().join(format!("wiparse_wfm_invalid_{}.wfm", std::process::id()));
        for (x, y) in [(vec![0.0], vec![]), (vec![0.0], vec![f64::NAN]),
            (vec![0.0, 1.0, 3.0], vec![1.0; 3]), (vec![1.0, 0.0], vec![1.0; 2])] {
            let trace = WaveformTrace { channel: "CH1".into(), x: x.into(), y: y.into(), x_unit: "s".into(), y_unit: "V".into() };
            assert!(export_waveform_wfm(&path, &trace).is_err());
            assert!(!path.exists());
        }
    }

    #[test]
    fn legacy_wiparse_wfm_fixed_header_remains_readable() {
        let mut bytes = vec![0; 900];
        bytes[..10].copy_from_slice(b"\x0f\x0fWFM#001\0");
        bytes[15] = 2;
        write_u32_le(&mut bytes, 16, 820);
        write_f64_le(&mut bytes, 174, 0.25);
        write_f64_le(&mut bytes, 182, -1.0);
        write_f64_le(&mut bytes, 478, 1e-6);
        write_u32_le(&mut bytes, 494, 3);
        write_u32_le(&mut bytes, 808, 6);
        bytes[820..826].copy_from_slice(&[0, 0, 4, 0, 8, 0]);
        let loaded = load_waveform_bytes(&bytes, "wfm", "CH1").unwrap();
        assert_eq!(&*loaded.y, &[-1.0, 0.0, 1.0]);
    }

    #[test]
    fn standard_wfm_rejects_unbacked_frame_counts_before_allocating_and_never_falls_back(){
        // These bytes are readable by the old fixed-offset dialect. Adding a
        // standard tag must not silently opt into those incompatible offsets.
        let mut bytes=vec![0;900];bytes[..10].copy_from_slice(b"\x0f\x0f:WFM#001");bytes[15]=2;
        write_u32_le(&mut bytes,16,820);write_f64_le(&mut bytes,174,0.25);
        write_f64_le(&mut bytes,182,-1.0);write_f64_le(&mut bytes,478,1e-6);
        write_u32_le(&mut bytes,494,3);write_u32_le(&mut bytes,808,6);
        bytes[820..826].copy_from_slice(&[0,0,4,0,8,0]);
        assert!(load_tek_wfm_legacy_fixed(&bytes,"CH1").is_ok());
        for count in [1u32,1000,u32::MAX]{
            write_u32_le(&mut bytes,150,count);
            let error=load_waveform_bytes_all(&bytes,"wfm","CH1").unwrap_err().to_string();
            assert!(error.contains("FastFrame"),"{error}");
        }
        write_u32_le(&mut bytes,150,0);write_u32_le(&mut bytes,16,800);
        assert!(load_waveform_bytes_all(&bytes,"wfm","CH1").is_err());
    }

    #[test]
    fn load_downloaded_tek_wfm_samples() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sample_waveforms/Tektronix_WFM");
        let names = ["AM_1Mhz.wfm", "analog_waveform.wfm", "data_test_waveform.wfm"];
        let mut loaded = 0usize;
        for name in names {
            let path = root.join(name);
            if !path.is_file() {
                continue;
            }
            let bytes = std::fs::read(&path).expect("read wfm");
            assert!(looks_like_wfm(&bytes), "{name} should look like Tek WFM");
            if bytes.len() < 830 {
                continue;
            }
            let trace = load_tek_wfm(&bytes, "CH1").unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!trace.x.is_empty(), "{name} empty");
            let (ymin, ymax) = trace.y.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
                (lo.min(v), hi.max(v))
            });
            assert!(
                (ymax - ymin).abs() > 1e-12,
                "{name}: flat/zero span ({ymin}..{ymax}), n={}",
                trace.y.len()
            );
            assert!(trace.y.len() >= 100, "{name}: unexpectedly few points {}", trace.y.len());
            loaded += 1;
        }
        // Samples are optional in some CI checkouts; skip if not present.
        if root.is_dir() {
            assert!(loaded >= 2, "expected at least two loadable WFM samples");
        }
    }

    #[test]
    fn load_generated_tek_4ch_wfm() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sample_waveforms/Tektronix_WFM/tek_4ch_sine_square_tri_saw.wfm");
        if !path.is_file() {
            return;
        }
        let traces = load_waveform_file_all(&path).expect("load 4ch wfm");
        assert_eq!(traces.len(), 4, "expected 4 channels");
        for (i, t) in traces.iter().enumerate() {
            assert_eq!(t.channel, format!("CH{}", i + 1));
            assert_eq!(t.y.len(), 2000);
            let (ymin, ymax) = t.y.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
                (lo.min(v), hi.max(v))
            });
            assert!((ymax - ymin).abs() > 0.1, "CH{} flat", i + 1);
        }
    }

    #[test]
    fn save_converts_csv_trace_to_isf() {
        let trace = WaveformTrace {
            channel: "CH1".into(),
            x: (0..64).map(|i| i as f64 * 1e-6).collect::<Vec<_>>().into(),
            y: (0..64).map(|i| (i as f64 * 0.1).sin()).collect::<Vec<_>>().into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let csv = waveform_to_spreadsheet_csv(&trace);
        let dir = std::env::temp_dir().join(format!("wiparse_conv_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("out.isf");
        save_waveform_file(&path, Some(&csv), Some("csv"), Some(&trace)).unwrap();
        let loaded = load_waveform_file(&path).unwrap();
        assert_eq!(loaded.y.len(), 64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rigol_wfm_4ch_folder() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sample_waveforms/Rigol_WFM_4ch");
        if !root.is_dir() {
            return;
        }
        let mut ok = 0usize;
        for entry in std::fs::read_dir(&root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("wfm")) != Some(true)
            {
                continue;
            }
            let traces = load_waveform_file_all(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(!traces.is_empty(), "{} empty", path.display());
            assert!(traces.iter().all(|t| t.y.len() >= 100), "{} short", path.display());
            ok += 1;
        }
        assert!(ok >= 3, "expected Rigol_WFM_4ch samples");
    }

    #[test]
    fn instrument_source_path_accepts_wfm001_and_wfm003() {
        // Same acceptance criteria as Tek FILESystem WFM capture → GUI WaveformSource.
        let wfm003 = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sample_waveforms/Tektronix_WFM/data_test_waveform.wfm");
        if wfm003.is_file() {
            let bytes = std::fs::read(&wfm003).unwrap();
            assert_eq!(sniff_waveform_ext(&bytes), Some("wfm"));
            assert!(bytes[2..10].starts_with(b":WFM#003"));
            let t = load_waveform_bytes(&bytes, "wfm", "CH1").unwrap();
            assert!(t.y.len() >= 100);
        }

        let trace = WaveformTrace {
            channel: "CH1".into(),
            x: (0..256).map(|i| i as f64 * 1e-6).collect::<Vec<_>>().into(),
            y: (0..256).map(|i| (i as f64 * 0.01).sin()).collect::<Vec<_>>().into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let dir = std::env::temp_dir().join(format!("wiparse_wfm_src_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("legacy001.wfm");
        export_waveform_wfm(&path, &trace).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[2..10], b":WFM#001");
        assert_eq!(sniff_waveform_ext(&bytes), Some("wfm"));
        let loaded = load_waveform_bytes(&bytes, "wfm", "CH1").unwrap();
        assert_eq!(loaded.y.len(), 256);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn square_wave(period_s: f64, duty: f64, samples: usize) -> WaveformTrace {
        let dt = period_s / 100.0;
        let y: Vec<f64> = (0..samples)
            .map(|i| {
                let t = (i as f64 * dt) % period_s;
                if t < period_s * duty {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        WaveformTrace {
            channel: "CH1".into(),
            x: (0..samples).map(|i| i as f64 * dt).collect::<Vec<_>>().into(),
            y: y.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        }
    }

    #[test]
    fn host_metrics_publish_measured_square_wave_and_omit_unmeasured() {
        let trace = square_wave(1e-3, 0.5, 400);
        let metrics = host_waveform_metrics(&[trace]);
        let ch1 = &metrics["CH1"];
        let freq = ch1["frequency_hz"]["value"].as_f64().unwrap();
        let duty = ch1["duty_cycle"]["value"].as_f64().unwrap();
        assert!((freq - 1000.0).abs() / 1000.0 < 0.05, "freq={freq}");
        assert!((duty - 0.5).abs() < 0.03, "duty={duty}");
        assert_eq!(ch1["frequency_hz"]["quality"], "good");
        assert_eq!(ch1["duty_cycle"]["unit"], "ratio");
        assert!(ch1.get("rise_time_s").is_none());
        assert!(ch1.get("fall_time_s").is_none());
    }

    #[test]
    fn host_metrics_omit_voltage_when_y_unit_is_not_volts() {
        let mut trace = square_wave(1e-3, 0.25, 400);
        trace.y_unit = "A".into();
        let ch1 = host_waveform_metrics(&[trace]);
        let ch1 = &ch1["CH1"];
        assert!(ch1.get("minimum_v").is_none());
        assert!(ch1.get("peak_to_peak_v").is_none());
        assert!(ch1.get("frequency_hz").is_some());
        assert!(ch1.get("duty_cycle").is_some());
    }

    #[test]
    fn estimate_duty_cycle_rejects_flat_trace() {
        assert_eq!(estimate_duty_cycle(&[1.0, 1.0, 1.0]), None);
        assert!((estimate_duty_cycle(&[0.0, 1.0, 0.0, 1.0]).unwrap() - 0.5).abs() < 1e-12);
    }
}

#[cfg(test)]mod allocation_budget_tests{
 use super::*;
 #[test]fn bounded_loader_stops_before_allocating_all_points_and_restores_legacy_calls(){
  let path=std::env::temp_dir().join(format!("wiparse-budget-{}.csv",std::process::id()));std::fs::write(&path,"x,y\n0,1\n1,2\n2,3\n").unwrap();
  assert!(load_waveform_file_bounded(&path,2).unwrap_err().to_string().contains("budget"));
  let accepted=load_waveform_file_bounded(&path,3).unwrap();assert_eq!(&*accepted[0].y,&[1.,2.,3.]);
  assert_eq!(load_waveform_file_all(&path).unwrap()[0].x.len(),3);std::fs::remove_file(path).unwrap();
 }
 #[test]fn header_point_claim_cannot_allocate_past_the_budget(){
  let previous=POINT_BUDGET.with(|b|b.replace(Some(2)));assert!(decode_curve_samples(&[1,2,3],1,"RI","LSB",usize::MAX).is_err());POINT_BUDGET.with(|b|b.set(previous));
 }
}
