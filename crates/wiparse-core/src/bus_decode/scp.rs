//! SCP single-wire NRZ, MSB first, odd parity, quarter-UI separators, CRC-8 0x39.
//! The default UI is 160 us; captured PINGs and separators track transmitter timing.
use super::charging::{append, byte_error, crc8, finish};
use super::digital::{LogicLevels, LogicWave};
use super::{
    BusByteError, BusByteSpan, BusDecodeResult, BusFrame, BusFrameError, MAX_DECODE_BYTES,
};
use crate::instrument::WaveformTrace;

#[derive(Debug, Clone)]
pub struct ScpConfig {
    pub ui_us: f64,
}
impl Default for ScpConfig {
    fn default() -> Self {
        Self { ui_us: 160.0 }
    }
}

pub fn decode_scp(
    trace: &WaveformTrace,
    threshold: Option<f64>,
    cfg: &ScpConfig,
) -> BusDecodeResult {
    let mut result = BusDecodeResult::default();
    if !cfg.ui_us.is_finite() || !(120.0..=200.0).contains(&cfg.ui_us) {
        result.error = Some("SCP UI must be 120..200 us".into());
        return result;
    }
    // SCP shares a wire between 1.8 V / 3.3 V transmitters. A midpoint
    // inferred from the largest swing can completely hide the 1.8 V sender.
    // Receiver limits in B.4.1 are VIL=0.9 V and VIH=1.3 V.
    let mid = threshold.filter(|t| t.is_finite()).unwrap_or(1.1);
    let wave = LogicWave::with_levels(trace, LogicLevels { vil: mid - 0.2, vih: mid + 0.2 });
    let edges = wave.edges();
    let mut ui = cfg.ui_us * 1e-6;
    let mut clock_acquired = false;
    let mut packet = Vec::new();
    let mut used = 0;
    let mut i = 0;
    while i + 1 < edges.len() && !result.truncated {
        let width = edges[i + 1].time - edges[i].time;
        if edges[i].level_after && (ui * 77.0..=ui * 123.0).contains(&width) {
            emit(&mut result, &mut used, &mut packet, false);
            if !append(&mut result, &mut used, BusFrame {
                t_start: edges[i].time, t_end: edges[i+1].time,
                summary: "RESET".into(), ..Default::default()
            }, vec![]) { break; }
            clock_acquired = false;
            i += 1;
            continue;
        }
        // Acquire transmitter timing from PING before applying per-byte timing.
        // The configured UI is only a fallback when no PING was captured.
        if edges[i].level_after && (16.0 * 120e-6..=16.0 * 200e-6).contains(&width) {
            emit(&mut result, &mut used, &mut packet, true);
            ui = width / 16.0;
            clock_acquired = true;
            if !append(
                &mut result,
                &mut used,
                BusFrame {
                    t_start: edges[i].time,
                    t_end: edges[i + 1].time,
                    summary: "PING".into(),
                    ..Default::default()
                },
                vec![],
            ) {
                break;
            }
            i += 1;
            continue;
        }
        if !(ui * 0.17..=ui * 0.33).contains(&width) {
            i += 1;
            continue;
        }
        let first = i;
        while i + 1 < edges.len()
            && (ui * 0.17..=ui * 0.33).contains(&(edges[i + 1].time - edges[i].time))
        {
            i += 1;
        }
        if i - first < 2 {
            continue;
        }
        let separator_ui = (edges[i].time - edges[first].time) / (i - first) as f64 * 4.0;
        // PING averages timing over 16 UI; quarter-UI edge quantization
        // otherwise accumulates across nine bits and skips the next separator.
        let bit = if clock_acquired && (separator_ui / ui - 1.0).abs() < 0.15 {
            ui
        } else { separator_ui };
        let start = edges[i].time;
        // An end separator is followed by a high 16-UI PING, not another byte.
        if i + 1 < edges.len() && edges[i].level_after && edges[i + 1].time - start > bit * 12.0 {
            continue;
        }
        let end = start + 9.0 * bit;
        if wave.at(start + 8.5 * bit).is_none() {
            break;
        }
        // A following separator must begin at the end of the parity bit. A
        // quarter-UI glitch inside data must not manufacture an apparently valid byte.
        let next_index = edges.partition_point(|e| e.time < end - bit * 0.3);
        let boundary_ok = edges
            .get(next_index)
            .is_some_and(|e| (e.time - end).abs() < bit * 0.3);
        let byte = (0..8).fold(0u8, |v, b| {
            (v << 1) | (wave.at(start + (b as f64 + 0.5) * bit) == Some(true)) as u8
        });
        let parity = wave.at(start + 8.5 * bit) == Some(true);
        let error = if !boundary_ok {
            BusByteError::Framing
        } else if (byte.count_ones() + u32::from(parity)) % 2 != 1 {
            BusByteError::Parity
        } else {
            BusByteError::None
        };
        packet.push(BusByteSpan {
            t_start: start,
            t_end: end,
            byte,
            error,
        });
        if packet.len() >= MAX_DECODE_BYTES {
            emit(&mut result, &mut used, &mut packet, false);
            result.truncated = true;
            break;
        }
        i = next_index.max(i + 1);
    }
    if !packet.is_empty() && !result.truncated {
        emit(&mut result, &mut used, &mut packet, false);
    }
    finish(&mut result, "SCP");
    result
}

fn emit(
    result: &mut BusDecodeResult,
    used: &mut usize,
    packet: &mut Vec<BusByteSpan>,
    complete: bool,
) {
    if packet.is_empty() {
        return;
    }
    let mut spans: Vec<BusByteSpan> = std::mem::take(packet);
    let bytes: Vec<u8> = spans.iter().map(|s| s.byte).collect();
    let error =
        if !complete || bytes.len() < 2 || spans.iter().any(|s| s.error == BusByteError::Framing) {
            BusFrameError::Framing
        } else if spans.iter().any(|s| s.error == BusByteError::Parity) {
            BusFrameError::Parity
        } else if crc8(&bytes[..bytes.len() - 1], 0x39) != bytes[bytes.len() - 1] {
            BusFrameError::Checksum
        } else {
            BusFrameError::None
        };
    if error == BusFrameError::Checksum {
        spans.last_mut().unwrap().error = byte_error(error);
    }
    let command = match bytes[0] {
        0x0b => "SBRWR",
        0x0c => "SBRRD",
        0x1b => "MBRWR",
        0x1c => "MBRRD",
        0x08 => "ACK",
        0x03 => "NACK",
        _ => "DATA",
    };
    let mut summary = format!("SCP {command}");
    if matches!(bytes[0], 0x0b | 0x0c | 0x1b | 0x1c) && bytes.len() > 2 {
        summary.push_str(&format!(" reg=0x{:02X}", bytes[1]));
    }
    if !complete {
        summary.push_str(" incomplete");
    }
    append(
        result,
        used,
        BusFrame {
            t_start: spans[0].t_start,
            t_end: spans.last().unwrap().t_end,
            summary,
            bytes,
            error,
        },
        spans,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    // Independent transmitter following B.3.3: at least three separator
    // edges, two rising edges at packet start, then 8 MSB bits + odd parity.
    fn packet_wave(y: &mut Vec<f64>, bytes: &[u8], high: f64, bad_parity: bool) {
        let mut previous = false;
        y.extend(std::iter::repeat_n(0.0, 64));
        for (index, &byte) in bytes.iter().enumerate() {
            let msb = byte & 0x80 != 0;
            let mut level = previous;
            for _ in 0..2 {
                level = !level;
                y.extend(std::iter::repeat_n(if level { high } else { 0.0 }, 8));
            }
            if level == msb {
                level = !level;
                y.extend(std::iter::repeat_n(if level { high } else { 0.0 }, 8));
            }
            for bit in (0..8).rev() {
                y.extend(std::iter::repeat_n(if byte >> bit & 1 != 0 { high } else { 0.0 }, 32));
            }
            previous = (byte.count_ones() % 2 == 0) ^ (bad_parity && index == 0);
            y.extend(std::iter::repeat_n(if previous { high } else { 0.0 }, 32));
        }
        // End separator terminates at a high 16 UI ping.
        if previous { y.extend(std::iter::repeat_n(0.0, 8)); }
        y.extend(std::iter::repeat_n(high, 8));
        y.extend(std::iter::repeat_n(0.0, 8));
        y.extend(std::iter::repeat_n(high, 16 * 32));
        y.extend(std::iter::repeat_n(0.0, 64));
    }

    fn waveform(y: Vec<f64>, ui: f64) -> WaveformTrace {
        WaveformTrace { channel: "D-".into(), x: (0..y.len()).map(|i| i as f64 * ui / 32.0).collect::<Vec<_>>().into(), y: y.into(), x_unit: "s".into(), y_unit: "V".into() }
    }

    #[test]
    fn all_bytes_and_mixed_voltage_senders_decode_from_waveform() {
        for ui in [144e-6, 160e-6, 176e-6] {
            let mut y = Vec::new();
            let mut bytes: Vec<u8> = (0..=255).collect();
            bytes.push(crc8(&bytes, 0x39));
            packet_wave(&mut y, &bytes, 1.8, false);
            packet_wave(&mut y, &[0x08, crc8(&[0x08], 0x39)], 3.3, false);
            let result = decode_scp(&waveform(y, ui), None, &ScpConfig::default());
            let data: Vec<_> = result.frames.iter().filter(|f| !f.bytes.is_empty()).collect();
            assert_eq!(data.len(), 2, "{result:?}");
            assert_eq!(data[0].bytes, bytes);
            assert!(data.iter().all(|f| f.error == BusFrameError::None), "{data:?}");
        }
    }

    #[test]
    fn physical_parity_and_crc_errors_are_reported() {
        for (bad_parity, crc, expected) in [(true, 0xc4, BusFrameError::Parity), (false, 0xc5, BusFrameError::Checksum)] {
            let mut y = Vec::new();
            packet_wave(&mut y, &[0x0c, 0x2b, 0x10, crc], 1.8, bad_parity);
            let result = decode_scp(&waveform(y, 160e-6), None, &ScpConfig::default());
            assert_eq!(result.frames[0].error, expected, "{result:?}");
        }
    }
    #[test]
    fn ping_clock_survives_quantized_separator_and_decodes_every_byte() {
        let mut y = vec![0.0; 100];
        y.extend(std::iter::repeat_n(1.8, 2548));
        y.extend(std::iter::repeat_n(0.0, 300));
        // Independent 1 us sampling: quarter pulses 40/41 us, data 159 us.
        let bytes = [0x08u8, 0x34, crc8(&[0x08, 0x34], 0x39)];
        let mut previous = false;
        for byte in bytes {
            let msb = byte & 0x80 != 0;
            for duration in [41, 40] {
                previous = !previous;
                y.extend(std::iter::repeat_n(if previous { 1.8 } else { 0.0 }, duration));
            }
            if previous == msb {
                previous = !previous;
                y.extend(std::iter::repeat_n(if previous { 1.8 } else { 0.0 }, 41));
            }
            for bit in (0..8).rev() {
                y.extend(std::iter::repeat_n(if byte >> bit & 1 != 0 { 1.8 } else { 0.0 }, 159));
            }
            previous = byte.count_ones() % 2 == 0;
            y.extend(std::iter::repeat_n(if previous { 1.8 } else { 0.0 }, 159));
        }
        if previous { y.extend(std::iter::repeat_n(0.0, 41)); }
        y.extend(std::iter::repeat_n(1.8, 40));
        y.extend(std::iter::repeat_n(0.0, 41));
        y.extend(std::iter::repeat_n(1.8, 2548));
        y.extend(std::iter::repeat_n(0.0, 100));
        let result = decode_scp(&waveform(y, 32e-6), None, &ScpConfig::default());
        let data: Vec<_> = result.frames.iter().filter(|f| !f.bytes.is_empty()).collect();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].bytes, bytes);
        assert_eq!(data[0].error, BusFrameError::None);
    }

    #[test]
    fn scp_crc_uses_0x39_polynomial() {
        assert_eq!(crc8(&[0x0c, 0x2b, 0x10], 0x39), 0xc4);
    }
    #[test]
    fn assembles_complete_command_with_crc() {
        let mut result = BusDecodeResult::default();
        let mut used = 0;
        let mut packet = [0x0c, 0x2b, 0x10, 0xc4]
            .into_iter()
            .enumerate()
            .map(|(i, byte)| BusByteSpan {
                t_start: i as f64,
                t_end: i as f64 + 1.0,
                byte,
                error: BusByteError::None,
            })
            .collect();
        emit(&mut result, &mut used, &mut packet, true);
        assert_eq!(result.frames[0].error, BusFrameError::None);
        assert_eq!(result.frames[0].summary, "SCP SBRRD reg=0x2B");
    }
    #[test]
    fn rejects_invalid_ui() {
        let trace = WaveformTrace {
            channel: "D+".into(),
            x: vec![0.0, 1.0].into(),
            y: vec![0.0, 3.3].into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let r = decode_scp(&trace, None, &ScpConfig { ui_us: 250.0 });
        assert!(r.error.unwrap().contains("120..200"));
    }

    #[test]
    fn recognizes_reset_pulse() {
        let mut y = vec![0.0; 64];
        y.extend(std::iter::repeat_n(1.8, 100 * 32));
        y.extend(std::iter::repeat_n(0.0, 64));
        let r = decode_scp(&waveform(y, 160e-6), None, &ScpConfig::default());
        assert_eq!(r.frames.len(), 1);
        assert_eq!(r.frames[0].summary, "RESET");
    }
    #[test]
    #[ignore = "Set WIPARSE_SCP_CAPTURE to the external Normal.isf capture"]
    fn real_normal_capture_regression() {
        let path = std::env::var("WIPARSE_SCP_CAPTURE").expect("external capture path");
        let traces = crate::waveform_file::load_waveform_file_all(&path).unwrap();
        let trace = traces.iter().find(|t| t.channel == "CH2").unwrap();
        let r = decode_scp(trace, None, &ScpConfig::default());
        assert_eq!(r.frames.len(), 278);
        assert!(r.frames.iter().all(|f| !f.has_error()), "{}", r.info);
        assert_eq!(r.frames.iter().filter(|f| !f.bytes.is_empty()).count(), 60);
        assert!(r.frames.iter().any(|f| f.bytes == [0x0c, 0x7e, 0x12] && f.summary == "SCP SBRRD reg=0x7E"));
    }
}
