//! Binary helpers for SCPI definite-length blocks and waveform downsampling.

/// Parse SCPI definite-length binary block (`#NXXXX<data>`).
pub fn parse_ieee_block(raw: &[u8]) -> &[u8] {
    if raw.is_empty() || raw[0] != b'#' {
        return raw;
    }
    if raw.len() < 2 {
        return raw;
    }
    let n_digits = (raw[1] as char).to_digit(10).unwrap_or(0) as usize;
    if n_digits == 0 || raw.len() < 2 + n_digits {
        return raw;
    }
    let len_str = std::str::from_utf8(&raw[2..2 + n_digits]).unwrap_or("0");
    let length: usize = len_str.parse().unwrap_or(0);
    let start = 2 + n_digits;
    let end = (start + length).min(raw.len());
    &raw[start..end]
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
    }

    #[test]
    fn ladder() {
        assert!((nearest_step(1.0, SCALE_STEPS_V, 1) - 2.0).abs() < 1e-9);
        assert!((nearest_step(1.0, SCALE_STEPS_V, -1) - 0.5).abs() < 1e-9);
    }
}
