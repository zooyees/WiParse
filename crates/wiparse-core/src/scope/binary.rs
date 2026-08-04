//! Binary helpers for SCPI definite-length blocks and waveform downsampling.

/// Max ASCII prefix before an IEEE `#N…` header (`:CURVE `, etc.).
/// Deliberately small so we never scan into PNG/binary payloads for a stray `0x23`.
const IEEE_PREFIX_MAX: usize = 64;

/// Locate a *leading* IEEE488.2 definite-length block header (`#NXXXX`).
///
/// Tek `CURVe?` may prefix with `:CURVE `; HARDCopy/`FILESystem:READFile` may
/// start at `#`. Must **not** search the whole buffer — PNG bytes frequently
/// contain `0x23` (`#`), which previously broke screenshot reads.
pub fn ieee_block_header_offset(raw: &[u8]) -> Option<usize> {
    let limit = raw.len().min(IEEE_PREFIX_MAX + 2);
    let mut i = 0usize;
    while i < limit {
        let b = raw[i];
        if b == b'#' {
            let n = *raw.get(i + 1)?;
            // Definite-length blocks use #1…#9 (digit count). #0 is indefinite.
            if (b'1'..=b'9').contains(&n) {
                return Some(i);
            }
            return None;
        }
        if b.is_ascii_whitespace()
            || b.is_ascii_alphanumeric()
            || matches!(b, b':' | b';' | b'_' | b'-' | b',' | b'.' | b'"' | b'\'')
        {
            i += 1;
            continue;
        }
        // Binary payload (e.g. raw PNG `\x89PNG…`) — not IEEE-framed.
        return None;
    }
    None
}

/// Total byte length of a SCPI payload that ends with a complete IEEE block
/// (`[prefix]#NXXXX<data>`), or `None` if the header is incomplete / absent.
pub fn ieee_block_total_len(raw: &[u8]) -> Option<usize> {
    let hash = ieee_block_header_offset(raw)?;
    if hash + 2 > raw.len() {
        return None;
    }
    let n_digits = (raw[hash + 1] as char).to_digit(10)? as usize;
    if !(1..=9).contains(&n_digits) || hash + 2 + n_digits > raw.len() {
        return None;
    }
    let len_str = std::str::from_utf8(&raw[hash + 2..hash + 2 + n_digits]).ok()?;
    let length: usize = len_str.parse().ok()?;
    Some(hash + 2 + n_digits + length)
}

/// True when `raw` contains a full IEEE definite-length block (payload present).
pub fn ieee_block_complete(raw: &[u8]) -> bool {
    match ieee_block_total_len(raw) {
        Some(need) => raw.len() >= need,
        None => false,
    }
}

/// True when `raw` looks like a finished PNG (IHDR…IEND).
pub fn png_complete(raw: &[u8]) -> bool {
    if raw.len() < 24 || !raw.starts_with(b"\x89PNG\r\n\x1a\n") {
        return false;
    }
    // Standard trailer: zero-length IEND chunk + CRC.
    raw.windows(8).any(|w| w == b"IEND\xaeB`\x82")
}

/// Parse SCPI definite-length binary block (`#NXXXX<data>`), skipping a short ASCII prefix.
/// If there is no valid leading IEEE header, returns `raw` unchanged (raw PNG path).
pub fn parse_ieee_block(raw: &[u8]) -> &[u8] {
    let Some(hash) = ieee_block_header_offset(raw) else {
        return raw;
    };
    let block = &raw[hash..];
    if block.len() < 2 {
        return raw;
    }
    let n_digits = (block[1] as char).to_digit(10).unwrap_or(0) as usize;
    if !(1..=9).contains(&n_digits) || block.len() < 2 + n_digits {
        return raw;
    }
    let len_str = std::str::from_utf8(&block[2..2 + n_digits]).unwrap_or("0");
    let length: usize = len_str.parse().unwrap_or(0);
    let start = 2 + n_digits;
    let end = (start + length).min(block.len());
    &block[start..end]
}

/// Uniform index decimation — strict time order, no min→max diagonals across buckets.
/// Best for zoomed polyline plots (square waves, digital edges).
pub fn decimate_uniform_index(x: &[f64], y: &[f64], max_points: usize) -> Vec<[f64; 2]> {
    let n = x.len().min(y.len());
    if n == 0 {
        return Vec::new();
    }
    let max_points = max_points.max(2);
    if n <= max_points {
        return x[..n]
            .iter()
            .zip(&y[..n])
            .map(|(&xv, &yv)| [xv, yv])
            .collect();
    }
    let mut out = Vec::with_capacity(max_points + 1);
    let step = n as f64 / max_points as f64;
    let mut f = 0.0;
    while out.len() < max_points {
        let idx = (f as usize).min(n - 1);
        out.push([x[idx], y[idx]]);
        f += step;
    }
    if out.last().map(|p| p[0]) != Some(x[n - 1]) {
        out.push([x[n - 1], y[n - 1]]);
    }
    out
}

/// Scope-style envelope: one vertical column per bucket (identical X for min & max).
/// Use with a vertical-segment painter — not a single connected polyline.
pub fn decimate_scope_envelope(x: &[f64], y: &[f64], max_columns: usize) -> Vec<[f64; 2]> {
    let n = x.len().min(y.len());
    if n == 0 {
        return Vec::new();
    }
    let max_columns = max_columns.max(1);
    let mut points = Vec::with_capacity(max_columns * 2);
    for b in 0..max_columns {
        let start = b * n / max_columns;
        let end = ((b + 1) * n / max_columns).max(start + 1).min(n);
        let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut any = false;
        for i in start..end {
            let v = y[i];
            if v.is_finite() {
                any = true;
                ymin = ymin.min(v);
                ymax = ymax.max(v);
            }
        }
        if !any {
            continue;
        }
        let x_mid = 0.5 * (x[start] + x[end - 1]);
        points.push([x_mid, ymin]);
        points.push([x_mid, ymax]);
    }
    points
}

/// Min/max downsampling for display (preserves peaks like Python `_downsample_minmax`).
pub fn downsample_minmax(x: &[f64], y: &[f64], target: usize) -> (Vec<f64>, Vec<f64>) {
    let n = x.len().min(y.len());
    if n == 0 || target == 0 || n <= target {
        return (x[..n].to_vec(), y[..n].to_vec());
    }
    let buckets = target / 2;
    if buckets == 0 {
        return (vec![x[0]], vec![y[0]]);
    }
    let mut ox = Vec::with_capacity(target);
    let mut oy = Vec::with_capacity(target);
    for b in 0..buckets {
        let start = b * n / buckets;
        let end = ((b + 1) * n / buckets).max(start + 1).min(n);
        let slice = &y[start..end];
        let (min_i, max_i) =
            slice
                .iter()
                .enumerate()
                .fold((0usize, 0usize), |(imin, imax), (i, v)| {
                    let imin = if *v < slice[imin] { i } else { imin };
                    let imax = if *v > slice[imax] { i } else { imax };
                    (imin, imax)
                });
        let (a, bidx) = if min_i <= max_i {
            (start + min_i, start + max_i)
        } else {
            (start + max_i, start + min_i)
        };
        ox.push(x[a]);
        oy.push(y[a]);
        if bidx != a {
            ox.push(x[bidx]);
            oy.push(y[bidx]);
        }
    }
    (ox, oy)
}

pub fn nearest_step(value: f64, steps: &[f64], direction: i32) -> f64 {
    if value <= 0.0 || steps.is_empty() {
        return steps.first().copied().unwrap_or(1e-3);
    }
    let mut best_i = 0usize;
    let mut best_d = f64::MAX;
    for (i, s) in steps.iter().enumerate() {
        let d = (*s - value).abs();
        if d < best_d {
            best_d = d;
            best_i = i;
        }
    }
    if direction > 0 {
        steps[(best_i + 1).min(steps.len() - 1)]
    } else if direction < 0 {
        steps[best_i.saturating_sub(1)]
    } else {
        steps[best_i]
    }
}

pub const SCALE_STEPS_V: &[f64] = &[
    1e-3, 2e-3, 5e-3, 10e-3, 20e-3, 50e-3, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0,
];

pub const SCALE_STEPS_S: &[f64] = &[
    1e-9, 2e-9, 5e-9, 10e-9, 20e-9, 50e-9, 100e-9, 200e-9, 500e-9, 1e-6, 2e-6, 5e-6, 10e-6, 20e-6,
    50e-6, 100e-6, 200e-6, 500e-6, 1e-3, 2e-3, 5e-3, 10e-3, 20e-3, 50e-3, 0.1, 0.2, 0.5, 1.0, 2.0,
    4.0, 10.0,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ieee_block() {
        let raw = b"#14\x01\x02\x03\x04XX";
        assert_eq!(parse_ieee_block(raw), &[1, 2, 3, 4]);
        let prefixed = b":CURVE #14\x01\x02\x03\x04";
        assert_eq!(parse_ieee_block(prefixed), &[1, 2, 3, 4]);
        assert!(ieee_block_complete(prefixed));
        assert!(!ieee_block_complete(b":CURVE #14\x01\x02"));
        assert_eq!(ieee_block_total_len(prefixed), Some(prefixed.len()));
    }

    #[test]
    fn ieee_header_ignores_hash_inside_png() {
        // PNG magic, then a later 0x23 that must NOT be treated as IEEE.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[0; 20]);
        png.push(b'#');
        png.extend_from_slice(b"512345garbage");
        assert_eq!(ieee_block_header_offset(&png), None);
        assert_eq!(parse_ieee_block(&png), png.as_slice());
        assert!(!ieee_block_complete(&png));
    }

    #[test]
    fn png_complete_detects_iend() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[0; 16]);
        assert!(!png_complete(&png));
        png.extend_from_slice(b"IEND\xaeB`\x82");
        assert!(png_complete(&png));
    }

    #[test]
    fn ladder() {
        assert!((nearest_step(1.0, SCALE_STEPS_V, 1) - 2.0).abs() < 1e-9);
        assert!((nearest_step(1.0, SCALE_STEPS_V, -1) - 0.5).abs() < 1e-9);
    }
}
