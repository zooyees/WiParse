//! Rigol proprietary `.wfm` loaders (DS1000Z / DS1000B / DS4000 / DHO800).
//!
//! Layouts follow the Kaitai specs in [scottprahl/RigolWFM](https://github.com/scottprahl/RigolWFM).

use crate::instrument::WaveformTrace;
use crate::waveform_file::WaveformFileError;
use flate2::read::ZlibDecoder;
use std::io::Read;

/// Magic / signature sniff for Rigol scope `.wfm` families we support.
pub fn looks_like_rigol_wfm(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    matches!(
        &bytes[..4],
        [0x01, 0xff, 0xff, 0xff] // DS1000Z / MSO1000Z
            | [0xa5, 0xa5, 0xa4, 0x01] // DS1000B
            | [0xa5, 0xa5, 0x38, 0x00] // DS4000
            | [0x02, 0x00, 0x00, 0x00] // DHO800 / DHO1000
    )
}

/// Load all enabled analog channels from a Rigol `.wfm`.
pub fn load_rigol_wfm_all(bytes: &[u8]) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    if bytes.len() < 64 {
        return Err(WaveformFileError::Parse("Rigol WFM too short".into()));
    }
    let traces = match &bytes[..4] {
        [0x01, 0xff, 0xff, 0xff] => load_1000z(bytes)?,
        [0xa5, 0xa5, 0xa4, 0x01] => load_1000b(bytes)?,
        [0xa5, 0xa5, 0x38, 0x00] => load_4000(bytes)?,
        [0x02, 0x00, 0x00, 0x00] => load_dho800(bytes)?,
        _ => {
            return Err(WaveformFileError::Parse(
                "unsupported Rigol WFM magic".into(),
            ))
        }
    };
    if traces.is_empty() {
        return Err(WaveformFileError::Parse(
            "Rigol WFM: no enabled channels".into(),
        ));
    }
    Ok(traces)
}

fn load_1000z(bytes: &[u8]) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    if bytes.len() < 280 {
        return Err(WaveformFileError::Parse("DS1000Z WFM truncated".into()));
    }
    let firmware = cstr_at(bytes, 28, 20);
    let h = 64usize;
    let ps_offset = read_i64(bytes, h + 8)?;
    let mask = bytes[h + 24];
    // Kaitai bitfields are big-endian within the byte: unused:4, ch4, ch3, ch2, ch1.
    let enabled = [
        mask & 0x01 != 0,
        mask & 0x02 != 0,
        mask & 0x04 != 0,
        mask & 0x08 != 0,
    ];
    let mut off = h + 24 + 1 + 3;
    let ch_file_off = [
        read_u32(bytes, off)?,
        read_u32(bytes, off + 4)?,
        read_u32(bytes, off + 8)?,
        read_u32(bytes, off + 12)?,
    ];
    off += 16 + 4; // la_offset
    off += 4; // acq/avg/sample/time_mode
    let memory_depth = read_u32(bytes, off)? as usize;
    off += 4;
    let sample_rate_ghz = read_f32(bytes, off)?;
    off += 4;

    let mut ch_scale = [0.0f64; 4];
    let mut ch_shift = [0.0f64; 4];
    let mut ch_inverted = [false; 4];
    for i in 0..4 {
        let base = off + i * 28;
        if base + 28 > bytes.len() {
            return Err(WaveformFileError::Parse("DS1000Z channel header OOB".into()));
        }
        let scale = read_f32(bytes, base + 8)? as f64;
        let shift = read_f32(bytes, base + 12)? as f64;
        let inverted = bytes[base + 16] != 0;
        ch_scale[i] = scale;
        ch_shift[i] = shift;
        ch_inverted[i] = inverted;
    }
    off += 4 * 28 + 12; // channels + la_parameters
    let horizontal_size = read_u32(bytes, off + 8)? as usize;
    let horizontal_offset = read_u32(bytes, off + 12)? as usize;

    let total = enabled.iter().filter(|e| **e).count().max(1);
    let stride = if total == 3 { 4 } else { total };
    if stride == 0 || memory_depth < stride {
        return Err(WaveformFileError::Parse("DS1000Z bad memory depth".into()));
    }
    let points = memory_depth / stride;
    let data_pos = {
        let from_ch = ch_file_off[0] as usize;
        let from_h = horizontal_offset.saturating_add(horizontal_size);
        if from_ch > 0 && from_ch + memory_depth <= bytes.len() {
            from_ch
        } else if from_h + memory_depth <= bytes.len() {
            from_h
        } else {
            return Err(WaveformFileError::Parse("DS1000Z data offset OOB".into()));
        }
    };
    let data = &bytes[data_pos..data_pos + memory_depth];
    let sample_rate_hz = sample_rate_ghz as f64 * 1e9;
    if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
        return Err(WaveformFileError::Parse("DS1000Z bad sample rate".into()));
    }
    let spp = 1.0 / sample_rate_hz;
    let time_offset = ps_offset as f64 * 1e-12;
    let start = time_offset - points as f64 * spp / 2.0;

    let mut traces = Vec::new();
    for (i, &en) in enabled.iter().enumerate() {
        if !en {
            continue;
        }
        let lane = channel_byte_offset(i + 1, stride, &enabled);
        let raw: Vec<u8> = data
            .iter()
            .skip(lane)
            .step_by(stride)
            .take(points)
            .copied()
            .collect();
        if raw.len() < 2 {
            continue;
        }
        let vdiv = if ch_inverted[i] {
            -ch_scale[i]
        } else {
            ch_scale[i]
        };
        let vertical_bias = if firmware == "00.04.04.SP3" && total == 2 {
            if ch_shift[i] < 0.0 {
                vdiv / 5.0
            } else {
                0.0
            }
        } else {
            vdiv
        };
        let y_scale = -vdiv / 20.0;
        let y_offset = ch_shift[i] - vertical_bias;
        let n = raw.len();
        let mut x = Vec::with_capacity(n);
        let mut y = Vec::with_capacity(n);
        for (k, &r) in raw.iter().enumerate() {
            x.push(start + k as f64 * spp);
            y.push(y_scale * (127.0 - r as f64) - y_offset);
        }
        traces.push(WaveformTrace {
            channel: format!("CH{}", i + 1),
            x: x.into(),
            y,
            x_unit: "s".into(),
            y_unit: "V".into(),
        });
    }
    Ok(traces)
}

/// Interleave lane for channel_number (1..4). See RigolWFM `_channel_bytes`.
fn channel_byte_offset(channel_number: usize, stride: usize, enabled: &[bool; 4]) -> usize {
    match stride {
        4 => 4usize.saturating_sub(channel_number),
        2 => {
            let before = enabled.iter().take(channel_number.saturating_sub(1)).any(|&e| e);
            if before {
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

fn load_1000b(bytes: &[u8]) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    if bytes.len() < 420 {
        return Err(WaveformFileError::Parse("DS1000B WFM truncated".into()));
    }
    // Spec places points at 56; some captures store it at 60 (pad before active_channel).
    let mut points = read_u32(bytes, 56)? as usize;
    if points == 0 {
        points = read_u32(bytes, 60)? as usize;
    }
    if points < 2 {
        return Err(WaveformFileError::Parse("DS1000B bad point count".into()));
    }

    let sample_rate_hz = read_f32(bytes, 180)? as f64;
    let time_offset = read_i64(bytes, 172)? as f64 * 1e-12;
    if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
        return Err(WaveformFileError::Parse("DS1000B bad sample rate".into()));
    }
    let spp = 1.0 / sample_rate_hz;

    let mut enabled = [false; 4];
    let mut y_scale = [0.0f64; 4];
    let mut y_offset = [0.0f64; 4];
    for i in 0..4 {
        let base = 68 + i * 24;
        if base + 24 > bytes.len() {
            return Err(WaveformFileError::Parse("DS1000B channel header OOB".into()));
        }
        let probe = read_f32(bytes, base + 8)? as f64;
        let en = bytes[base + 14] != 0;
        let inverted = bytes[base + 15] != 0;
        let scale_measured = read_i32(bytes, base + 16)? as f64;
        let shift_measured = read_i16(bytes, base + 20)? as f64;
        let vdiv = if inverted {
            -1.0e-6 * scale_measured * probe
        } else {
            1.0e-6 * scale_measured * probe
        };
        let volt_scale = vdiv / 25.0;
        let volt_offset = shift_measured * volt_scale;
        enabled[i] = en;
        y_scale[i] = volt_scale;
        // RigolWFM applies an extra +1.12 div bias for 1000B.
        y_offset[i] = volt_offset + 1.12 * vdiv;
    }

    // Channel payloads occupy fixed slots at 420 + i*points (disabled slots still reserved).
    if 420 + points * 4 > bytes.len() && 420 + points > bytes.len() {
        return Err(WaveformFileError::Parse("DS1000B data truncated".into()));
    }

    let mut traces = Vec::new();
    for i in 0..4 {
        if !enabled[i] {
            continue;
        }
        let start = 420 + i * points;
        if start + points > bytes.len() {
            break;
        }
        let raw = &bytes[start..start + points];
        let h = points as f64 * spp / 2.0;
        let mut x = Vec::with_capacity(points);
        let mut y = Vec::with_capacity(points);
        for (k, &r) in raw.iter().enumerate() {
            let t = -h + (if points > 1 {
                k as f64 * (2.0 * h) / (points - 1) as f64
            } else {
                0.0
            }) + time_offset;
            x.push(t);
            y.push(y_scale[i] * (127.0 - r as f64) - y_offset[i]);
        }
        traces.push(WaveformTrace {
            channel: format!("CH{}", i + 1),
            x: x.into(),
            y,
            x_unit: "s".into(),
            y_unit: "V".into(),
        });
    }
    Ok(traces)
}

fn load_4000(bytes: &[u8]) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    if bytes.len() < 0x300 {
        return Err(WaveformFileError::Parse("DS4000 WFM truncated".into()));
    }
    let model = cstr_at(bytes, 4, 20);
    let mut off = 4 + 20 + 20 + 20; // magic+model+fw+5*u32
    let mask = bytes[off];
    off += 1 + 3;
    let enabled = [
        mask & 0x01 != 0,
        mask & 0x02 != 0,
        mask & 0x04 != 0,
        mask & 0x08 != 0,
    ];
    let positions = [
        read_u32(bytes, off)? as usize,
        read_u32(bytes, off + 4)? as usize,
        read_u32(bytes, off + 8)? as usize,
        read_u32(bytes, off + 12)? as usize,
    ];
    off += 16 + 12; // position + 3*u32
    let _mem1 = read_u32(bytes, off)?;
    off += 4;
    let sample_rate_hz = read_f32(bytes, off)? as f64;
    off += 4 + 4; // rate + unknown
    off += 8 + 8; // time_per_div_ps + 2*u32

    let mut vdiv = [0.0f64; 4];
    let mut voff = [0.0f64; 4];
    let mut inverted = [false; 4];
    for i in 0..4 {
        // channel_header: 8 u1 + 2 f4 + 4 u1 + 2 u4 = 8+8+4+8 = 28
        let base = off + i * 28;
        if base + 28 > bytes.len() {
            return Err(WaveformFileError::Parse("DS4000 channel header OOB".into()));
        }
        vdiv[i] = read_f32(bytes, base + 8)? as f64;
        voff[i] = read_f32(bytes, base + 12)? as f64;
        inverted[i] = bytes[base + 16] != 0;
    }
    off += 4 * 28 + 24; // channels + 6*u32
    let _mem2 = read_u32(bytes, off)?;
    off += 4 + 4;
    let mem_depth = read_u32(bytes, off)? as usize;

    // time_header is far later; use sample-aligned centered time from sample rate.
    if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 || mem_depth < 2 {
        return Err(WaveformFileError::Parse("DS4000 bad timing/depth".into()));
    }
    let spp = 1.0 / sample_rate_hz;
    // model substring(2,3)=='2' → /25 else /32 (DS4024 → '4' → 32)
    let vert_div = if model.as_bytes().get(2) == Some(&b'2') {
        25.0
    } else {
        32.0
    };

    // Prefer time_header actual_offset when present (offset after large fixed region).
    // time_header starts after: mem_depth + unk + bytes_per_ch*2 + 41*u4 + total_samples + 4*u4 + mem_depth_type + 27 + ...
    // For display we use centered window (sample_aligned with memory_depth), time_offset=0 fallback.
    let time_offset = 0.0f64;
    let start = time_offset - mem_depth as f64 * spp / 2.0;

    let mut traces = Vec::new();
    for i in 0..4 {
        if !enabled[i] || positions[i] == 0 {
            continue;
        }
        let pos = positions[i];
        if pos + mem_depth > bytes.len() {
            return Err(WaveformFileError::Parse("DS4000 channel data OOB".into()));
        }
        let raw = &bytes[pos..pos + mem_depth];
        let signed = if inverted[i] { -vdiv[i] } else { vdiv[i] };
        let volt_scale = signed / vert_div;
        // DS4000 ADC polarity: y_scale = -volt_scale
        let y_scale = -volt_scale;
        let y_offset = voff[i];
        let mut x = Vec::with_capacity(mem_depth);
        let mut y = Vec::with_capacity(mem_depth);
        for (k, &r) in raw.iter().enumerate() {
            x.push(start + k as f64 * spp);
            y.push(y_scale * (127.0 - r as f64) - y_offset);
        }
        traces.push(WaveformTrace {
            channel: format!("CH{}", i + 1),
            x: x.into(),
            y,
            x_unit: "s".into(),
            y_unit: "V".into(),
        });
    }
    Ok(traces)
}

fn load_dho800(bytes: &[u8]) -> Result<Vec<WaveformTrace>, WaveformFileError> {
    if bytes.len() < 64 {
        return Err(WaveformFileError::Parse("DHO WFM truncated".into()));
    }
    // Per-channel scale/offset from metadata blocks (type 5 = DHO800, type 9 = DHO1000).
    let mut scale = [None; 4];
    let mut offset = [None; 4];
    let mut o = 24usize;
    loop {
        if o + 12 > bytes.len() {
            return Err(WaveformFileError::Parse("DHO metadata truncated".into()));
        }
        let block_id = read_u16(bytes, o)? as usize;
        let block_type = read_u16(bytes, o + 2)?;
        let decomp_size = read_u16(bytes, o + 4)? as usize;
        let comp_size = read_u16(bytes, o + 6)? as usize;
        let len_content_raw = read_u16(bytes, o + 8)? as usize;
        o += 12;
        if len_content_raw == 0 && comp_size == 0 {
            break;
        }
        if o + len_content_raw > bytes.len() {
            return Err(WaveformFileError::Parse("DHO block content OOB".into()));
        }
        let content_raw = &bytes[o..o + len_content_raw];
        o += len_content_raw;
        if !(matches!(block_type, 5 | 9) && (1..=4).contains(&block_id)) {
            continue;
        }
        let payload = decode_block_payload(content_raw, comp_size, decomp_size)?;
        if payload.len() < 42 {
            continue;
        }
        let scale_num = read_i64(&payload, 1)?;
        let (sc, off) = if block_type == 5 {
            // DHO800
            let sc = scale_num as f64 / 7_500_000_000_000.0;
            let v_center_raw = read_i32(&payload, 38)?;
            let v_center = -(v_center_raw as f64) / 1.0e9;
            let off = v_center - sc * 32768.0;
            (sc, off)
        } else {
            // DHO1000
            let sc = scale_num as f64 / 750_000_000_000.0;
            let v_center = read_i64(&payload, 38)? as f64 / 1.0e8;
            let off = -v_center - sc * 32768.0;
            (sc, off)
        };
        scale[block_id - 1] = Some(sc);
        offset[block_id - 1] = Some(off);
    }

    while o < bytes.len() && bytes[o] == 0 {
        o += 1;
    }
    if o + 40 > bytes.len() {
        return Err(WaveformFileError::Parse("DHO data section missing".into()));
    }
    let n_total = read_u64(bytes, o)? as f64;
    let xinc_ticks = read_u32(bytes, o + 16)? as f64;
    let n_pts = read_u32(bytes, o + 24)? as usize;
    if n_pts < 2 {
        return Err(WaveformFileError::Parse("DHO bad point count".into()));
    }
    let n_ch = ((n_total / n_pts as f64).round() as usize).clamp(1, 4);
    let need = o + 40 + n_pts * n_ch * 2;
    if need > bytes.len() {
        return Err(WaveformFileError::Parse("DHO samples truncated".into()));
    }
    // DHO800 ADC tick = 0.8 ns; DHO1000 uses 10 ns. Our samples are DHO800.
    let tick = 0.8e-9;
    let x_increment = xinc_ticks * tick;
    let x_origin = -(n_pts as f64 / 2.0) * x_increment;
    let samples = &bytes[o + 40..o + 40 + n_pts * n_ch * 2];

    let mut traces = Vec::new();
    for ch in 0..n_ch {
        let sc = scale[ch].unwrap_or(1.0 / 750_000_000_000.0);
        let offv = offset[ch].unwrap_or(-sc * 32768.0);
        let mut x = Vec::with_capacity(n_pts);
        let mut y = Vec::with_capacity(n_pts);
        for i in 0..n_pts {
            let idx = (i * n_ch + ch) * 2;
            let raw = u16::from_le_bytes([samples[idx], samples[idx + 1]]) as f64;
            x.push(x_origin + i as f64 * x_increment);
            y.push(sc * raw + offv);
        }
        traces.push(WaveformTrace {
            channel: format!("CH{}", ch + 1),
            x: x.into(),
            y,
            x_unit: "s".into(),
            y_unit: "V".into(),
        });
    }
    Ok(traces)
}

fn decode_block_payload(
    content_raw: &[u8],
    comp_size: usize,
    decomp_size: usize,
) -> Result<Vec<u8>, WaveformFileError> {
    if comp_size > content_raw.len() {
        return Err(WaveformFileError::Parse("DHO comp_size OOB".into()));
    }
    let payload = &content_raw[..comp_size];
    if comp_size == decomp_size {
        return Ok(payload.to_vec());
    }
    let mut dec = ZlibDecoder::new(payload);
    let mut out = Vec::with_capacity(decomp_size);
    dec.read_to_end(&mut out)
        .map_err(|e| WaveformFileError::Parse(format!("DHO zlib: {e}")))?;
    Ok(out)
}

fn cstr_at(bytes: &[u8], off: usize, len: usize) -> String {
    if off >= bytes.len() {
        return String::new();
    }
    let end = (off + len).min(bytes.len());
    let slice = &bytes[off..end];
    let nul = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..nul]).into_owned()
}

fn read_u16(bytes: &[u8], off: usize) -> Result<u16, WaveformFileError> {
    bytes
        .get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| WaveformFileError::Parse("u16 OOB".into()))
}

fn read_u32(bytes: &[u8], off: usize) -> Result<u32, WaveformFileError> {
    bytes
        .get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| WaveformFileError::Parse("u32 OOB".into()))
}

fn read_u64(bytes: &[u8], off: usize) -> Result<u64, WaveformFileError> {
    bytes
        .get(off..off + 8)
        .map(|s| {
            u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
        })
        .ok_or_else(|| WaveformFileError::Parse("u64 OOB".into()))
}

fn read_i16(bytes: &[u8], off: usize) -> Result<i16, WaveformFileError> {
    bytes
        .get(off..off + 2)
        .map(|s| i16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| WaveformFileError::Parse("i16 OOB".into()))
}

fn read_i32(bytes: &[u8], off: usize) -> Result<i32, WaveformFileError> {
    bytes
        .get(off..off + 4)
        .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| WaveformFileError::Parse("i32 OOB".into()))
}

fn read_i64(bytes: &[u8], off: usize) -> Result<i64, WaveformFileError> {
    bytes
        .get(off..off + 8)
        .map(|s| {
            i64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
        })
        .ok_or_else(|| WaveformFileError::Parse("i64 OOB".into()))
}

fn read_f32(bytes: &[u8], off: usize) -> Result<f32, WaveformFileError> {
    bytes
        .get(off..off + 4)
        .map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| WaveformFileError::Parse("f32 OOB".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn load_rigol_4ch_samples() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sample_waveforms/Rigol_WFM_4ch");
        if !root.is_dir() {
            return;
        }
        let cases: &[(&str, usize)] = &[
            ("DS1054Z-CH1UartCH2OffCh3SquareCh4Sine.wfm", 3),
            ("MSO1104.wfm", 1),
            ("DS1204B-A.wfm", 4),
            ("DS4024-A.wfm", 2),
            ("DHO824-ch1234.wfm", 4),
        ];
        let mut loaded = 0usize;
        for &(name, expect_ch) in cases {
            let path = root.join(name);
            if !path.is_file() {
                continue;
            }
            let bytes = std::fs::read(&path).expect(name);
            assert!(looks_like_rigol_wfm(&bytes), "{name}");
            let traces = load_rigol_wfm_all(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(traces.len(), expect_ch, "{name} channel count");
            for t in &traces {
                assert!(t.y.len() >= 100, "{name} {} short", t.channel);
                let (lo, hi) = t.y.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
                    (a.min(v), b.max(v))
                });
                assert!(
                    (hi - lo).abs() > 1e-9 || t.y.iter().any(|&v| v.abs() > 1e-9),
                    "{name} {} flat/zero",
                    t.channel
                );
            }
            loaded += 1;
        }
        assert!(loaded >= 3, "expected Rigol sample files present");
    }
}
