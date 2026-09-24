//! USB PD CC: BMC, 4b5b, SOP ordered sets and CRC-32/ISO-HDLC.
use super::charging::{append, byte_error, finish, signal};
use super::{BusByteSpan, BusDecodeResult, BusFrame, BusFrameError};
use crate::instrument::WaveformTrace;

const CODES: [u8; 16] = [
    30, 9, 20, 21, 10, 11, 14, 15, 18, 19, 22, 23, 26, 27, 28, 29,
];
const ORDERED: [([u8; 4], &str); 7] = [
    ([24, 24, 24, 17], "SOP"),
    ([24, 24, 6, 6], "SOP'"),
    ([24, 6, 24, 6], "SOP''"),
    ([24, 25, 25, 6], "SOP' Debug"),
    ([24, 25, 6, 17], "SOP'' Debug"),
    ([7, 7, 7, 25], "Hard Reset"),
    ([7, 24, 7, 6], "Cable Reset"),
];

#[derive(Clone, Copy)]
struct Bit {
    value: bool,
    start: f64,
    end: f64,
}

pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    !crc
}

fn symbol(bits: &[Bit], pos: usize) -> Option<u8> {
    let group = bits.get(pos..pos + 5)?;
    Some(
        group
            .iter()
            .enumerate()
            .fold(0, |v, (i, b)| v | ((b.value as u8) << i)),
    )
}

pub fn decode_usb_pd(trace: &WaveformTrace, threshold: Option<f64>) -> BusDecodeResult {
    let edges = signal(trace, threshold).edges();
    let mut result = BusDecodeResult::default();
    let mut used = 0;
    // Estimate UI from plausible full and half intervals near the 300 kbit/s rate.
    let mut periods: Vec<f64> = edges
        .windows(2)
        .filter_map(|e| {
            let dt = e[1].time - e[0].time;
            if (2.7e-6..4.1e-6).contains(&dt) {
                Some(dt)
            } else if (1.3e-6..2.1e-6).contains(&dt) {
                Some(dt * 2.0)
            } else {
                None
            }
        })
        .take(4096)
        .collect();
    periods.sort_by(f64::total_cmp);
    let ui = periods
        .get(periods.len() / 2)
        .copied()
        .unwrap_or(1.0 / 300_000.0);
    let near = |dt: f64, expected: f64| (dt - expected).abs() < ui * 0.22;
    let mut bits = Vec::new();
    let mut i = 0;
    while i + 1 < edges.len() {
        let dt = edges[i + 1].time - edges[i].time;
        let consumed = if near(dt, ui) {
            1
        } else if near(dt, ui / 2.0)
            && i + 2 < edges.len()
            && near(edges[i + 2].time - edges[i + 1].time, ui / 2.0)
        {
            2
        } else {
            0
        };
        if consumed == 0 {
            parse_bits(&bits, &mut result, &mut used);
            bits.clear();
            i += 1;
        } else {
            bits.push(Bit {
                value: consumed == 2,
                start: edges[i].time,
                end: edges[i + consumed].time,
            });
            i += consumed;
        }
        if result.truncated {
            break;
        }
    }
    if !result.truncated {
        parse_bits(&bits, &mut result, &mut used);
    }
    finish(&mut result, "USB PD");
    result
}

fn parse_bits(bits: &[Bit], result: &mut BusDecodeResult, used: &mut usize) {
    let mut pos = 0;
    while pos + 20 <= bits.len() {
        let ordered = ORDERED
            .iter()
            .find(|(codes, _)| (0..4).all(|i| symbol(bits, pos + i * 5) == Some(codes[i])));
        let Some((_, name)) = ordered else {
            pos += 1;
            continue;
        };
        let start = pos;
        pos += 20;
        if name.contains("Reset") {
            if !append(
                result,
                used,
                BusFrame {
                    t_start: bits[start].start,
                    t_end: bits[pos - 1].end,
                    summary: (*name).into(),
                    ..Default::default()
                },
                vec![],
            ) {
                return;
            }
            continue;
        }
        let mut bytes = Vec::new();
        let mut spans = Vec::new();
        let mut eop = false;
        while pos + 5 <= bits.len() && bytes.len() <= 268 {
            if symbol(bits, pos) == Some(13) {
                pos += 5;
                eop = true;
                break;
            }
            if bytes.len() == 268 { break; }
            let nibble = |p| {
                symbol(bits, p)
                    .and_then(|s| CODES.iter().position(|&v| v == s))
                    .map(|v| v as u8)
            };
            let (Some(lo), Some(hi)) = (nibble(pos), nibble(pos + 5)) else {
                break;
            };
            let byte = lo | hi << 4;
            bytes.push(byte);
            spans.push(BusByteSpan {
                t_start: bits[pos].start,
                t_end: bits[pos + 9].end,
                byte,
                error: Default::default(),
            });
            pos += 10;
        }
        let mut error = BusFrameError::Framing;
        let mut summary = format!("{name} incomplete");
        if bytes.len() >= 2 {
            let header = u16::from_le_bytes([bytes[0], bytes[1]]);
            let count = ((header >> 12) & 7) as usize;
            summary = format!(
                "{name} {} ID={} NDO={count}",
                message_name(header),
                (header >> 9) & 7
            );
            if eop && packet_length(header, &bytes) == Some(bytes.len()) {
                let at = bytes.len() - 4;
                let received = u32::from_le_bytes(bytes[at..].try_into().unwrap());
                error = if crc32(&bytes[..at]) == received {
                    BusFrameError::None
                } else {
                    BusFrameError::Checksum
                };
                if error == BusFrameError::Checksum {
                    for s in &mut spans[at..] {
                        s.error = byte_error(error);
                    }
                }
                if header & 0x8000 == 0 && header & 31 == 1 && count > 0 {
                    for data in bytes[2..at].chunks_exact(4) {
                        let pdo = u32::from_le_bytes(data.try_into().unwrap());
                        if pdo >> 30 == 0 {
                            summary.push_str(&format!(
                                " {:.2}V/{:.2}A",
                                ((pdo >> 10) & 1023) as f64 * 0.05,
                                (pdo & 1023) as f64 * 0.01
                            ));
                        } else if pdo >> 28 == 12 {
                            summary.push_str(&format!(
                                " PPS {:.1}-{:.1}V/{:.2}A",
                                ((pdo >> 8) & 255) as f64 * 0.1,
                                ((pdo >> 17) & 255) as f64 * 0.1,
                                (pdo & 127) as f64 * 0.05
                            ));
                        }
                    }
                }
                if header & 0x8000 != 0 {
                    summary.push_str(" (extended payload, raw)");
                }
            }
        }
        if error == BusFrameError::Framing {
            for s in &mut spans {
                s.error = byte_error(error);
            }
        }
        if !append(
            result,
            used,
            BusFrame {
                t_start: bits[start].start,
                t_end: bits[pos.saturating_sub(1)].end,
                summary,
                bytes,
                error,
            },
            spans,
        ) {
            return;
        }
    }
}

// USB-IF PD 3.0 tests E13/E18: unchunked uses Data Size, not NDO.
fn packet_length(header: u16, bytes: &[u8]) -> Option<usize> {
    let count = ((header >> 12) & 7) as usize;
    if header & 0x8000 == 0 { return Some(6 + count * 4); }
    let ext = u16::from_le_bytes([*bytes.get(2)?, *bytes.get(3)?]);
    let size = (ext & 0x1ff) as usize;
    if size > 260 { return None; }
    if ext & 0x8000 == 0 {
        if count != 0 || ext & 0x7c00 != 0 { return None; }
        Some(8 + size)
    } else if count > 0 {
        Some(6 + count * 4)
    } else { None }
}

fn message_name(header: u16) -> &'static str {
    if header & 0x8000 != 0 {
        return "Extended";
    }
    let kind = header & 31;
    if header & 0x7000 != 0 {
        match kind {
            1 => "Source_Capabilities",
            2 => "Request",
            3 => "BIST",
            4 => "Sink_Capabilities",
            15 => "Vendor_Defined",
            _ => "Data",
        }
    } else {
        match kind {
            1 => "GoodCRC",
            2 => "GotoMin",
            3 => "Accept",
            4 => "Reject",
            5 => "Ping",
            6 => "PS_RDY",
            7 => "Get_Source_Cap",
            8 => "Get_Sink_Cap",
            9 => "DR_Swap",
            10 => "PR_Swap",
            11 => "VCONN_Swap",
            12 => "Wait",
            13 => "Soft_Reset",
            16 => "Not_Supported",
            _ => "Control",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn add_symbol(out: &mut Vec<bool>, symbol: u8) {
        for i in 0..5 {
            out.push(symbol >> i & 1 != 0);
        }
    }
    fn trace(header: u16, bad_crc: bool) -> WaveformTrace {
        let mut bytes = header.to_le_bytes().to_vec();
        let mut crc = crc32(&bytes).to_le_bytes();
        if bad_crc {
            crc[0] ^= 1;
        }
        bytes.extend(crc);
        let mut bits = Vec::new();
        for s in [24, 24, 24, 17] {
            add_symbol(&mut bits, s);
        }
        for b in bytes {
            add_symbol(&mut bits, CODES[(b & 15) as usize]);
            add_symbol(&mut bits, CODES[(b >> 4) as usize]);
        }
        add_symbol(&mut bits, 13);
        bits.push(false);
        let spb = 12usize;
        let ui = 1.0 / 300_000.0;
        let mut level = false;
        let mut y = vec![0.0; spb];
        for one in bits {
            level = !level;
            y.extend(std::iter::repeat_n(if level { 1.0 } else { 0.0 }, spb / 2));
            if one {
                level = !level;
            }
            y.extend(std::iter::repeat_n(if level { 1.0 } else { 0.0 }, spb / 2));
        }
        WaveformTrace {
            channel: "CC1".into(),
            x: (0..y.len())
                .map(|i| i as f64 * ui / spb as f64)
                .collect::<Vec<_>>()
                .into(),
            y: y.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        }
    }
    #[test]
    fn decodes_goodcrc() {
        let r = decode_usb_pd(&trace(1, false), None);
        assert_eq!(r.frames.len(), 1, "{r:?}");
        assert!(r.frames[0].summary.contains("GoodCRC"));
        assert_eq!(r.frames[0].error, BusFrameError::None);
    }
    #[test]
    fn detects_crc_error() {
        let r = decode_usb_pd(&trace(1, true), None);
        assert_eq!(r.frames[0].error, BusFrameError::Checksum);
    }

    #[test]
    fn crc32_matches_standard_check_vector() { assert_eq!(crc32(b"123456789"), 0xcbf43926); }
    #[test]
    fn truncated_packet_is_not_accepted() {
        let mut w = trace(1, false);
        let n = w.x.len() * 3 / 4;
        w.x = w.x[..n].to_vec().into(); w.y = w.y[..n].to_vec().into();
        let r = decode_usb_pd(&w, None);
        assert!(r.frames.iter().any(|f| f.error == BusFrameError::Framing), "{r:?}");
    }


    #[test]
    fn ordered_sets_and_fixed_source_capabilities() {
        for (codes, name) in ORDERED {
            let mut raw = Vec::new();
            for code in codes { add_symbol(&mut raw, code); }
            if !name.contains("Reset") {
                let mut bytes = vec![0x01, 0x10, 0x2C, 0x91, 0x01, 0x00]; // 5 V, 3 A PDO
                bytes.extend(crc32(&bytes).to_le_bytes());
                for byte in bytes { add_symbol(&mut raw, CODES[(byte & 15) as usize]); add_symbol(&mut raw, CODES[(byte >> 4) as usize]); }
                add_symbol(&mut raw, 13);
            }
            let bits: Vec<_> = raw.into_iter().enumerate().map(|(i, value)| Bit { value, start: i as f64, end: i as f64 + 1.0 }).collect();
            let mut r = BusDecodeResult::default(); let mut used = 0;
            parse_bits(&bits, &mut r, &mut used);
            assert_eq!(r.frames.len(), 1, "{name}: {r:?}");
            assert!(!r.frames[0].has_error(), "{r:?}");
            assert!(r.frames[0].summary.starts_with(name));
            if !name.contains("Reset") { assert!(r.frames[0].summary.contains("5.00V/3.00A")); }
        }
    }

    #[test]
    fn unchunked_extended_payload_including_maximum_length() {
        for size in [1usize, 24, 260] {
            let mut bytes = vec![1, 0x80];
            bytes.extend((size as u16).to_le_bytes());
            bytes.extend(std::iter::repeat_n(0x55, size));
            bytes.extend(crc32(&bytes).to_le_bytes());
            let mut raw = Vec::new();
            for code in [24, 24, 24, 17] { add_symbol(&mut raw, code); }
            for &byte in &bytes { add_symbol(&mut raw, CODES[(byte & 15) as usize]); add_symbol(&mut raw, CODES[(byte >> 4) as usize]); }
            add_symbol(&mut raw, 13);
            let bits: Vec<_> = raw.into_iter().enumerate().map(|(i, value)| Bit { value, start: i as f64, end: i as f64 + 1.0 }).collect();
            let mut r = BusDecodeResult::default(); let mut used = 0;
            parse_bits(&bits, &mut r, &mut used);
            assert_eq!(r.frames.len(), 1);
            assert_eq!(r.frames[0].bytes, bytes);
            assert!(!r.frames[0].has_error(), "{r:?}");
        }
    }
}
