//! UFCS UART 8N1 physical layer and length-delimited messages (SOP 0xAA).
use super::charging::{append, byte_error, crc8, finish, signal};
use super::{BusByteSpan, BusDecodeResult, BusFrame, BusFrameError, EdgeKind, MAX_DECODE_BYTES};
use crate::instrument::WaveformTrace;

#[derive(Debug, Clone)]
pub struct UfcsConfig {
    pub baud: u32,
}
impl Default for UfcsConfig {
    fn default() -> Self {
        Self { baud: 115_200 }
    }
}

pub fn decode_ufcs(
    trace: &WaveformTrace,
    threshold: Option<f64>,
    cfg: &UfcsConfig,
) -> BusDecodeResult {
    let mut result = BusDecodeResult::default();
    if ![38_400, 57_600, 115_200, 230_400].contains(&cfg.baud) {
        result.error = Some("UFCS baud must be 38400, 57600, 115200 or 230400".into());
        return result;
    }
    let wave = signal(trace, threshold);
    let bit = 1.0 / cfg.baud as f64;
    let mut next = f64::NEG_INFINITY;
    let mut packet: Vec<BusByteSpan> = Vec::new();
    let mut used = 0;
    for edge in wave.edges() {
        if edge.kind != EdgeKind::Falling || edge.time < next {
            continue;
        }
        let start = edge.time;
        if wave.at(start + 0.5 * bit) != Some(false) {
            continue;
        }
        let Some(stop) = wave.at(start + 9.5 * bit) else {
            break;
        };
        let byte = (0..8).fold(0, |value, i| {
            value | ((wave.at(start + (1.5 + i as f64) * bit) == Some(true)) as u8) << i
        });
        next = start + 9.8 * bit;
        let span = BusByteSpan {
            t_start: start,
            t_end: start + 10.0 * bit,
            byte,
            error: if stop {
                Default::default()
            } else {
                super::BusByteError::Framing
            },
        };
        if packet.last().is_some_and(|b| start - b.t_end > 5e-3) {
            emit(&mut result, &mut used, &mut packet, false);
        }
        if result.truncated {
            break;
        }
        if packet.is_empty() && byte != 0xaa {
            continue;
        }
        packet.push(span);
        // SOP inside a payload is data, not a new packet.
        let expected = if packet.len() >= 4 {
            match packet[2].byte & 7 {
                0 => Some(5),
                1 if packet.len() >= 5 => Some(6 + packet[4].byte as usize),
                2 if packet.len() >= 6 => Some(7 + packet[5].byte as usize),
                1 | 2 => None,
                _ => Some(packet.len()),
            }
        } else {
            None
        };
        if let Some(total) = expected {
            if packet.len() >= total {
                emit(&mut result, &mut used, &mut packet, true);
            }
        }
        if used >= MAX_DECODE_BYTES || result.truncated {
            result.truncated = true;
            break;
        }
    }
    if !packet.is_empty() && !result.truncated {
        emit(&mut result, &mut used, &mut packet, false);
    }
    finish(&mut result, "UFCS");
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
    let mut spans = std::mem::take(packet);
    let bytes: Vec<u8> = spans.iter().map(|s| s.byte).collect();
    let mut error = BusFrameError::Framing;
    let mut summary = "UFCS incomplete".to_string();
    if bytes.len() >= 4 {
        let header = u16::from_be_bytes([bytes[1], bytes[2]]);
        let kind = header & 7;
        let target = match (header >> 13) & 7 {
            1 => "Source",
            2 => "Sink",
            3 => "Cable",
            _ => "Reserved",
        };
        summary = format!(
            "{} →{target} ID={} Rev={}",
            name(kind, bytes[3]),
            (header >> 9) & 15,
            (header >> 3) & 63
        );
        if complete
            && kind <= 2
            && spans.iter().all(|s| s.error == super::BusByteError::None)
            && bytes.len() >= 5
        {
            error = if crc8(&bytes[1..bytes.len() - 1], 0x29) == bytes[bytes.len() - 1] {
                BusFrameError::None
            } else {
                BusFrameError::Checksum
            };
        }
        if kind == 1 && bytes[3] == 2 && bytes.len() == 14 {
            let request = u64::from_be_bytes(bytes[5..13].try_into().unwrap());
            summary.push_str(&format!(
                " mode={} {:.2}V/{:.2}A",
                request >> 60,
                ((request >> 16) & 65535) as f64 * 0.01,
                (request & 65535) as f64 * 0.01
            ));
        }
    }
    if error == BusFrameError::Checksum {
        spans.last_mut().unwrap().error = byte_error(error);
    } else if error == BusFrameError::Framing {
        for s in &mut spans {
            s.error = byte_error(error);
        }
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

fn name(kind: u16, command: u8) -> &'static str {
    if kind == 0 {
        match command {
            0 => "PING",
            1 => "ACK",
            2 => "NCK",
            3 => "ACCEPT",
            4 => "SOFT_RESET",
            5 => "POWER_READY",
            6 => "GET_OUTPUT_CAPABILITIES",
            7 => "GET_SOURCE_INFO",
            8 => "GET_SINK_INFO",
            9 => "GET_CABLE_INFO",
            10 => "GET_DEVICE_INFO",
            11 => "GET_ERROR_INFO",
            12 => "DETECT_CABLE_INFO",
            13 => "START_CABLE_DETECT",
            14 => "END_CABLE_DETECT",
            15 => "EXIT_UFCS_MODE",
            _ => "Control (reserved)",
        }
    } else if kind == 1 {
        match command {
            1 => "OUTPUT_CAPABILITIES",
            2 => "REQUEST",
            3 => "SOURCE_INFO",
            4 => "SINK_INFO",
            5 => "CABLE_INFO",
            6 => "DEVICE_INFO",
            7 => "ERROR_INFO",
            8 => "CONFIG_WATCHDOG",
            9 => "REFUSE",
            10 => "VERIFY_REQUEST",
            11 => "VERIFY_RESPONSE",
            _ => "Data (reserved)",
        }
    } else if kind == 2 {
        "Vendor_Defined"
    } else {
        "Reserved message type"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn uart(bytes: &[u8], baud: u32) -> WaveformTrace {
        let spb = 16usize;
        let dt = 1.0 / baud as f64 / spb as f64;
        let mut y = vec![3.3; spb];
        for &b in bytes {
            y.extend(std::iter::repeat_n(0.0, spb));
            for bit in 0..8 {
                y.extend(std::iter::repeat_n(
                    if b >> bit & 1 != 0 { 3.3 } else { 0.0 },
                    spb,
                ));
            }
            y.extend(std::iter::repeat_n(3.3, spb));
        }
        y.extend(std::iter::repeat_n(3.3, spb));
        WaveformTrace {
            channel: "D+".into(),
            x: (0..y.len())
                .map(|i| i as f64 * dt)
                .collect::<Vec<_>>()
                .into(),
            y: y.into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        }
    }
    #[test]
    fn decodes_control_packet_and_crc() {
        let mut bytes = vec![0xaa, 0x20, 0x00, 0x01];
        bytes.push(crc8(&bytes[1..], 0x29));
        let r = decode_ufcs(&uart(&bytes, 115_200), None, &UfcsConfig::default());
        assert_eq!(r.frames.len(), 1, "{r:?}");
        assert_eq!(r.frames[0].summary, "ACK →Source ID=0 Rev=0");
        assert_eq!(r.frames[0].error, BusFrameError::None);
    }
    #[test]
    fn marks_bad_crc() {
        let bytes = [0xaa, 0x20, 0x00, 0x01, 0xff];
        let r = decode_ufcs(&uart(&bytes, 115_200), None, &UfcsConfig::default());
        assert_eq!(r.frames[0].error, BusFrameError::Checksum);
    }

    #[test]
    fn data_with_sop_in_payload_at_all_bauds() {
        let mut bytes = vec![0xaa, 0x20, 0x01, 0x08, 2, 0xaa, 0x55];
        bytes.push(crc8(&bytes[1..], 0x29));
        for baud in [38_400, 57_600, 115_200, 230_400] {
            let r = decode_ufcs(&uart(&bytes, baud), None, &UfcsConfig { baud });
            assert_eq!(r.frames.len(), 1);
            assert_eq!(r.frames[0].bytes, bytes);
            assert!(!r.frames[0].has_error(), "{r:?}");
        }
    }
    #[test]
    fn incomplete_payload_is_framing_error() {
        let r = decode_ufcs(&uart(&[0xaa, 0x20, 1, 8, 2, 0xaa], 115_200), None, &UfcsConfig::default());
        assert_eq!(r.frames.len(), 1);
        assert_eq!(r.frames[0].error, BusFrameError::Framing);
    }

}
