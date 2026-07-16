//! Light Qi log-line parse (ASK/FSK hex extract + header name).

use super::decode::decode_qi_message;
use super::defs::{ask_packet_name, fsk_bare_name, fsk_packet_name, get_payload_len};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QiLineParse {
    pub raw: String,
    pub direction: Option<String>,
    pub bytes: Vec<u8>,
    pub header: Option<u8>,
    pub name: Option<String>,
    pub payload_len: Option<u8>,
    pub known: bool,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decode: Option<super::decode::QiDecode>,
}

fn extract_hex_bytes(segment: &str) -> Vec<u8> {
    let cleaned = segment.replace("0x", "").replace(',', " ");
    cleaned
        .split_whitespace()
        .filter_map(|tok| {
            let t = tok.trim();
            if t.is_empty() || !t.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            u8::from_str_radix(t, 16).ok()
        })
        .collect()
}

/// Parse a single `TX0:` / ASK / FSK style log line.
pub fn parse_qi_line(line: &str) -> QiLineParse {
    if let Some(d) = decode_qi_message(line) {
        return QiLineParse {
            raw: d.raw.clone(),
            direction: Some(d.direction.clone()),
            bytes: d.bytes.clone(),
            header: Some(d.header),
            name: Some(d.name.clone()),
            payload_len: Some(get_payload_len(d.header)),
            known: d.known,
            summary: d.summary.clone(),
            decode: Some(d),
        };
    }

    let raw = line.trim().to_string();
    let upper = raw.to_ascii_uppercase();

    let (direction, start_pat) = if upper.contains("ASK ") {
        ("ASK", "ASK ")
    } else if upper.contains("FSK ") {
        ("FSK", "FSK ")
    } else {
        return QiLineParse {
            raw,
            direction: None,
            bytes: vec![],
            header: None,
            name: None,
            payload_len: None,
            known: false,
            summary: "not an ASK/FSK line".into(),
            decode: None,
        };
    };

    let start = upper
        .find(start_pat)
        .map(|i| i + start_pat.len())
        .unwrap_or(0);
    let rest = &raw[start.min(raw.len())..];
    let end = rest
        .find(" F ")
        .or_else(|| {
            if direction == "FSK" {
                rest.find('(')
            } else {
                None
            }
        })
        .unwrap_or(rest.len());
    let hex_part = &rest[..end];
    let bytes = extract_hex_bytes(hex_part);

    if bytes.is_empty() {
        return QiLineParse {
            raw,
            direction: Some(direction.into()),
            bytes,
            header: None,
            name: None,
            payload_len: None,
            known: false,
            summary: format!("{direction}: empty payload"),
            decode: None,
        };
    }

    let header = bytes[0];
    let (name, known) = if direction == "ASK" {
        (
            ask_packet_name(header).map(|s| s.to_string()),
            ask_packet_name(header).is_some(),
        )
    } else if bytes.len() == 1 {
        (
            fsk_bare_name(header).map(|s| s.to_string()),
            fsk_bare_name(header).is_some(),
        )
    } else {
        (
            fsk_packet_name(header)
                .or_else(|| fsk_bare_name(header))
                .map(|s| s.to_string()),
            fsk_packet_name(header).is_some() || fsk_bare_name(header).is_some(),
        )
    };

    let payload_len = Some(get_payload_len(header));
    let name_s = name.clone().unwrap_or_else(|| format!("0x{header:02X}"));
    let summary = format!(
        "{direction} {name_s} header=0x{header:02X} bytes={} known={known}",
        bytes.len()
    );

    QiLineParse {
        raw,
        direction: Some(direction.into()),
        bytes,
        header: Some(header),
        name,
        payload_len,
        known,
        summary,
        decode: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ask_ept() {
        let p = parse_qi_line("TX0:[12:00:00.000] ASK 02 01 F ");
        assert_eq!(p.direction.as_deref(), Some("ASK"));
        assert_eq!(p.header, Some(0x02));
        assert_eq!(p.name.as_deref(), Some("EPT"));
        assert!(p.known);
        assert!(p.decode.is_some());
    }
}
