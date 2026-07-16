//! Structured Qi field decode (JSON-friendly, mirrors Python `parse_message_dict`).

use super::defs::{
    ask_packet, bpp_grq_request, ept_reason, fod_type_label, fsk_bare_name, fsk_depth_label,
    fsk_packet, get_payload_len, mpp_grq_request, msr_main_mode, msr_pref, prmc_vendor,
    rp_mode_label, srq_type_name,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QiField {
    pub name: String,
    pub value: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// Table 88 — PTx Simultaneous Data Stream Response type.
fn sdsr_type_name(code: u8) -> &'static str {
    match code {
        0 => "ACK",
        1 => "UNEXPECTED",
        2 => "ERR_BUSY",
        3 => "ERR_CRC",
        _ => "Reserved",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QiDecode {
    pub raw: String,
    pub direction: String,
    pub header: u8,
    pub name: String,
    pub desc: String,
    pub profile: String,
    pub bytes: Vec<u8>,
    pub payload: Vec<u8>,
    pub payload_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum_ok: Option<bool>,
    pub fields: Vec<QiField>,
    pub known: bool,
    pub summary: String,
}

fn field(
    name: &str,
    value: impl Into<serde_json::Value>,
    unit: &str,
    raw: Option<String>,
) -> QiField {
    QiField {
        name: name.into(),
        value: value.into(),
        unit: if unit.is_empty() {
            None
        } else {
            Some(unit.into())
        },
        raw,
    }
}

fn s8(v: u8) -> i8 {
    v as i8
}

fn u16_be(hi: u8, lo: u8) -> u16 {
    ((hi as u16) << 8) | lo as u16
}

fn s16_be(hi: u8, lo: u8) -> i16 {
    u16_be(hi, lo) as i16
}

fn bytes_fields(p: &[u8]) -> Vec<QiField> {
    p.iter()
        .enumerate()
        .map(|(i, b)| field(&format!("byte_{i}"), *b, "", Some(format!("0x{b:02X}"))))
        .collect()
}

fn add_remaining(fields: &mut Vec<QiField>, p: &[u8], start: usize) {
    fields.extend(
        bytes_fields(&p[start.min(p.len())..])
            .into_iter()
            .enumerate()
            .map(|(i, mut value)| {
                value.name = format!("byte_{}", start + i);
                value
            }),
    );
}

fn dsr_name(code: u8) -> &'static str {
    match code {
        0 => "DSR/ack — ACK",
        1 => "DSR/poll — POLL",
        2 => "DSR/nak — NAK",
        3 => "DSR/nd — ND",
        _ => "Reserved",
    }
}

fn get_name(code: u8) -> &'static str {
    match code {
        0 => "PTx Extended Identification (XID)",
        2 => "PTx Inverter Voltage (INV)",
        3 => "PTx PLA Parameters (PLAP)",
        4 => "PTx Extended Capabilities (ECAP)",
        5 => "PTx Regulation Control Status",
        6 => "PTx Charge Status",
        7 => "PTx Estimated K",
        9 => "PTx Error Status",
        10 => "PTx Power Modes Capabilities",
        11 => "PTx Extended Power Modes Capabilities",
        12 => "PTx Mated-Q Results",
        13 => "PTx PLA_2 Parameters",
        15 => "PTx Gain Measurement Parameters",
        16 => "PTx Calibration Capabilities",
        17 => "PTx dPLoss Calibration Parameters",
        _ => "Reserved / undefined",
    }
}

fn sadc_name(code: u8) -> &'static str {
    match code {
        0 => "Reset all streams",
        1 => "Reset stream transport",
        2 => "Close and abort transport",
        3 => "Close and complete transport",
        4 => "Open transport",
        _ => "Reserved",
    }
}

fn hex_bytes(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split body into payload + optional XOR checksum (same rules as Python).
pub fn split_payload_checksum(header: u8, body: &[u8]) -> (Vec<u8>, Option<u8>, Option<bool>) {
    if body.is_empty() {
        return (vec![], None, None);
    }
    let msg_len = get_payload_len(header) as usize;
    if body.len() == msg_len {
        return (body.to_vec(), None, None);
    }
    if body.len() >= msg_len + 1 {
        let payload = body[..msg_len].to_vec();
        let cs = body[msg_len];
        let mut calc = header;
        for b in &payload {
            calc ^= *b;
        }
        return (payload, Some(cs), Some(calc == cs));
    }
    (body.to_vec(), None, None)
}

fn hex_region_end(rest: &str) -> usize {
    if let Some(i) = rest.find(" F ") {
        return i;
    }
    // Trailing " F" terminator (no space after F) — must not match inside " FE".
    let trimmed = rest.trim_end();
    if let Some(i) = trimmed.rfind(" F") {
        if trimmed[i + 2..].is_empty() {
            return i;
        }
    }
    if let Some(i) = rest.find('(') {
        return i;
    }
    rest.len()
}

fn extract_hex_region(line: &str) -> Option<(String, &'static str)> {
    let upper = line.to_ascii_uppercase();
    if let Some(i) = upper.find("ASK ") {
        let start = i + 4;
        let rest = &line[start.min(line.len())..];
        let end = hex_region_end(rest);
        return Some((rest[..end].trim().to_string(), "ASK"));
    }
    if let Some(i) = upper.find("FSK ") {
        let start = i + 4;
        let rest = &line[start.min(line.len())..];
        let end = hex_region_end(rest);
        return Some((rest[..end].trim().to_string(), "FSK"));
    }
    None
}

fn parse_hex_bytes(segment: &str) -> Vec<u8> {
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

fn decode_ask_fields(header: u8, p: &[u8]) -> Vec<QiField> {
    match header {
        0x00 if !p.is_empty() => vec![
            field("reserved", p[0], "", Some(format!("0x{:02X}", p[0]))),
            field("valid", p[0] == 0, "", None),
        ],
        0x01 if !p.is_empty() => vec![
            field("signal_strength", p[0], "", Some(format!("0x{:02X}", p[0]))),
            field(
                "signal_strength_pct",
                (p[0] as f64 / 255.0 * 100.0 * 10.0).round() / 10.0,
                "%",
                None,
            ),
        ],
        0x02 if !p.is_empty() => {
            let reason = ept_reason(p[0])
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Reserved 0x{:02X}", p[0]));
            vec![
                field("reason_code", p[0], "", Some(format!("0x{:02X}", p[0]))),
                field("reason", reason, "", None),
            ]
        }
        0x54 | 0x55 if p.len() >= 2 => {
            let data_len = p.len() - 2;
            let crc = u16_be(p[data_len], p[data_len + 1]);
            let mut fields = vec![
                field("wpid_segment", hex_bytes(&p[..data_len]), "", None),
                field("crc16", crc, "", Some(format!("0x{crc:04X}"))),
            ];
            add_remaining(&mut fields, p, 0);
            fields
        }
        0x38 if p.len() >= 3 => vec![
            field("selector", p[0], "", Some(format!("0x{:02X}", p[0]))),
            field(
                "stream_number",
                p[1] & 0x0F,
                "",
                Some(format!("0x{:02X}", p[1])),
            ),
            field(
                "response_type",
                p[2] & 0x0F,
                "",
                Some(format!("0x{:02X}", p[2])),
            ),
            field(
                "response_desc",
                match p[2] & 0x0F {
                    0 => "ACK",
                    1 => "UNEXPECTED",
                    2 => "ERR_BUSY",
                    3 => "ERR_CRC",
                    _ => "Reserved",
                },
                "",
                None,
            ),
        ],
        0x48 if p.len() >= 4 => {
            let req = p[0] & 7;
            let parameter = u16_be(p[2], p[3]);
            vec![
                field("request", req, "", Some(format!("0x{:02X}", p[0]))),
                field("request_desc", sadc_name(req), "", None),
                field(
                    "stream_number",
                    p[1] & 0x1F,
                    "",
                    Some(format!("0x{:02X}", p[1])),
                ),
                field(
                    "parameter",
                    parameter,
                    "",
                    Some(format!("0x{parameter:04X}")),
                ),
            ]
        }
        0x50 if p.len() >= 3 => vec![
            field("selector", p[0] & 1, "", Some(format!("0x{:02X}", p[0]))),
            field(
                "alpha_0r",
                s8(p[1]) as f64 / 100.0,
                "",
                Some(format!("0x{:02X}", p[1])),
            ),
            field(
                "alpha_1r",
                s8(p[2]) as f64 / 100.0,
                "",
                Some(format!("0x{:02X}", p[2])),
            ),
        ],
        0x58 if p.len() >= 5 => {
            let selector = p[0] >> 5;
            if selector == 0 {
                let report_id = p[0] & 3;
                let mut fields = vec![
                    field("selector", selector, "", Some(format!("0x{:02X}", p[0]))),
                    field("report_id", report_id, "", None),
                    field(
                        "report_type",
                        if report_id == 2 {
                            "PRx Identification"
                        } else {
                            "Reserved"
                        },
                        "",
                        None,
                    ),
                ];
                if report_id == 2 {
                    let random_id = (u32::from(p[1] & 0x3F) << 14)
                        | (u32::from(p[2]) << 6)
                        | u32::from(p[3] >> 2);
                    fields.push(field(
                        "random_identifier",
                        random_id,
                        "",
                        Some(format!("0x{random_id:05X}")),
                    ));
                } else {
                    fields.push(field("report_data", hex_bytes(&p[1..]), "", None));
                }
                fields
            } else if selector == 1 {
                let received_power = u16_be(p[1], p[2]);
                let p_rect = u16_be(p[3], p[4]);
                vec![
                    field("selector", selector, "", Some(format!("0x{:02X}", p[0]))),
                    field(
                        "received_power",
                        received_power,
                        "mW",
                        Some(format!("0x{received_power:04X}")),
                    ),
                    field("p_rect", p_rect, "mW", Some(format!("0x{p_rect:04X}"))),
                ]
            } else {
                vec![
                    field("selector", selector, "", Some(format!("0x{:02X}", p[0]))),
                    field("data", hex_bytes(&p[1..]), "", None),
                ]
            }
        }
        0x03 if !p.is_empty() => vec![field(
            "control_error",
            s8(p[0]) as i64,
            "",
            Some(format!("0x{:02X}", p[0])),
        )],
        0x04 if !p.is_empty() => vec![
            field("received_power", p[0], "", Some(format!("0x{:02X}", p[0]))),
            field(
                "received_power_pct",
                ((p[0] as f64 / 128.0) * 1000.0).round() / 10.0,
                "% MaxPower",
                None,
            ),
        ],
        0x05 if !p.is_empty() => vec![
            field(
                "charge_status",
                if p[0] <= 100 {
                    serde_json::json!(p[0])
                } else {
                    serde_json::Value::Null
                },
                if p[0] <= 100 { "%" } else { "" },
                Some(format!("0x{:02X}", p[0])),
            ),
            field(
                "charge_status_desc",
                if p[0] == 0xFE {
                    "Battery level temporarily not available"
                } else if p[0] == 0xFF {
                    "Device does not have a battery"
                } else if p[0] <= 100 {
                    "Available"
                } else {
                    "Reserved"
                },
                "",
                None,
            ),
        ],
        0x06 if !p.is_empty() => {
            vec![field(
                "hold_off_time",
                p[0],
                "ms",
                Some(format!("0x{:02X}", p[0])),
            )]
        }
        0x07 if !p.is_empty() => {
            let req = bpp_grq_request(p[0])
                .or_else(|| mpp_grq_request(p[0]))
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Request header 0x{:02X}", p[0]));
            vec![
                field("req_header", p[0], "", Some(format!("0x{:02X}", p[0]))),
                field("request", req, "", None),
            ]
        }
        0x13 if !p.is_empty() => {
            let pref = (p[0] >> 6) & 0x03;
            let main_mode = (p[0] >> 3) & 0x03;
            let aux = p[0] & 0x01;
            vec![
                field("mode_cfg", p[0], "", Some(format!("0x{:02X}", p[0]))),
                field("preference", msr_pref(pref), "", None),
                field("main_mode", msr_main_mode(main_mode), "", None),
                field(
                    "auxiliary_mode",
                    if aux == 1 { "Selected" } else { "Not selected" },
                    "",
                    None,
                ),
            ]
        }
        0x15 if !p.is_empty() => vec![
            field("response_code", p[0], "", Some(format!("0x{:02X}", p[0]))),
            field("response", dsr_name(p[0]), "", None),
        ],
        0x19 if !p.is_empty() => vec![field(
            "extended_ce",
            s8(p[0]) as i64,
            "/100",
            Some(format!("0x{:02X}", p[0])),
        )],
        0x20 if !p.is_empty() => {
            let mut fields = vec![
                field("request_code", p[0], "", Some(format!("0x{:02X}", p[0]))),
                field("request_type", srq_type_name(p[0]), "", None),
            ];
            if p.len() >= 2 {
                fields.push(field(
                    "parameter",
                    p[1],
                    "",
                    Some(format!("0x{:02X}", p[1])),
                ));
            }
            fields
        }
        0x23 if !p.is_empty() => {
            let mut fields = vec![
                field("operation", p[0], "", Some(format!("0x{:02X}", p[0]))),
                field(
                    "operation_desc",
                    if p[0] == 1 {
                        "COMMIT (commit calibration result)"
                    } else {
                        "Reserved"
                    },
                    "",
                    None,
                ),
            ];
            add_remaining(&mut fields, p, 1);
            fields
        }
        0x25 if !p.is_empty() => {
            let mut fields = vec![field("request", p[0], "", Some(format!("0x{:02X}", p[0])))];
            add_remaining(&mut fields, p, 1);
            fields
        }
        0x28 if p.len() >= 2 => vec![
            field("reserved", p[0], "", Some(format!("0x{:02X}", p[0]))),
            field("parameter", p[1], "", Some(format!("0x{:02X}", p[1]))),
            field("request_type", get_name(p[1]), "", None),
        ],
        0x29 if p.len() >= 2 => {
            let mask = u16_be(p[0], p[1]);
            let streams = (0..16)
                .filter(|bit| mask & (1 << bit) != 0)
                .collect::<Vec<_>>();
            vec![
                field("streams_mask", mask, "", Some(format!("0x{mask:04X}"))),
                field("enabled_streams", serde_json::json!(streams), "", None),
            ]
        }
        0x2C => {
            let mut fields = vec![field("calibration_action", "Enter calibration", "", None)];
            add_remaining(&mut fields, p, 0);
            fields
        }
        0x2D if !p.is_empty() => {
            let mut fields = vec![
                field("clear_flag", p[0] & 1, "", Some(format!("0x{:02X}", p[0]))),
                field(
                    "calibration_points",
                    if p[0] & 1 != 0 { "Clear" } else { "Keep" },
                    "",
                    None,
                ),
            ];
            add_remaining(&mut fields, p, 1);
            fields
        }
        0x22 if !p.is_empty() => {
            let t = p[0] & 0x01;
            let mut fields = vec![field(
                "fod_type",
                fod_type_label(t),
                "",
                Some(format!("0x{:02X}", p[0])),
            )];
            if p.len() >= 2 {
                fields.push(field(
                    "fod_value",
                    p[1],
                    "",
                    Some(format!("0x{:02X}", p[1])),
                ));
            }
            fields
        }
        0x31 if p.len() >= 3 => {
            let mode = p[0] & 0x07;
            let mw = u16_be(p[1], p[2]);
            vec![
                field(
                    "mode",
                    rp_mode_label(mode),
                    "",
                    Some(format!("0x{:02X}", p[0])),
                ),
                field("received_power", mw, "mW", Some(format!("0x{mw:04X}"))),
            ]
        }
        0x51 if p.len() >= 5 => {
            let ref_pwr = p[0] & 0x3F;
            let ai = (p[2] >> 6) & 1;
            let ob = (p[2] >> 4) & 1;
            let count = p[2] & 0x07;
            let win_size = (p[3] >> 3) & 0x1F;
            let win_offset = p[3] & 0x07;
            let neg = (p[4] >> 7) & 1;
            let polarity = (p[4] >> 6) & 1;
            let depth = (p[4] >> 4) & 0x03;
            let buf_size = (p[4] >> 1) & 0x07;
            let dup = p[4] & 0x01;
            vec![
                field("ref_power", ref_pwr, "", Some(format!("0x{:02X}", p[0]))),
                field("ref_power_w", (ref_pwr as f64) * 0.5, "W", None),
                field("ai", if ai == 1 { "Supported" } else { "No" }, "", None),
                field("ob", if ob == 1 { "Supported" } else { "No" }, "", None),
                field("optional_packet_count", count, "", None),
                field("window_size", win_size, "", None),
                field("window_offset", win_offset, "", None),
                field("profile", if neg == 1 { "EPP" } else { "BPP" }, "", None),
                field(
                    "fsk_polarity",
                    if polarity == 1 {
                        "Negative"
                    } else {
                        "Positive"
                    },
                    "",
                    None,
                ),
                field("fsk_depth", fsk_depth_label(depth), "", None),
                field("buffer_size", buf_size, "", None),
                field("dup", if dup == 1 { "Supported" } else { "No" }, "", None),
            ]
        }
        0x71 if p.len() >= 3 => {
            let major = p[0] >> 4;
            let minor = p[0] & 0x0F;
            let mfg = u16_be(p[1], p[2]);
            let mut fields = vec![
                field(
                    "qi_version",
                    format!("{major}.{minor}"),
                    "",
                    Some(format!("0x{:02X}", p[0])),
                ),
                field("prmc", mfg, "", Some(format!("0x{mfg:04X}"))),
            ];
            if let Some(v) = prmc_vendor(mfg) {
                fields.push(field("vendor", v, "", None));
            }
            if p.len() >= 7 {
                let ext = (p[3] >> 7) & 1;
                let basic_id = (u32::from(p[3] & 0x7F) << 24)
                    | (u32::from(p[4]) << 16)
                    | (u32::from(p[5]) << 8)
                    | u32::from(p[6]);
                fields.push(field(
                    "has_xid",
                    if ext == 1 { "Has XID" } else { "No XID" },
                    "",
                    None,
                ));
                fields.push(field(
                    "basic_device_id",
                    format!("0x{basic_id:07X}"),
                    "",
                    None,
                ));
            }
            fields
        }
        0x78 if p.len() >= 7 => vec![
            field("reserved", p[0], "", Some(format!("0x{:02X}", p[0]))),
            field(
                "alpha_fm",
                s16_be(p[1], p[2]),
                "raw",
                Some(format!("0x{:02X}{:02X}", p[1], p[2])),
            ),
            field("alpha_fm_mohm", s16_be(p[1], p[2]) as f64 * 0.5, "mΩ", None),
            field(
                "alpha_fm_dc",
                s16_be(p[3], p[4]),
                "raw",
                Some(format!("0x{:02X}{:02X}", p[3], p[4])),
            ),
            field(
                "alpha_fm_dc_mw",
                s16_be(p[3], p[4]) as f64 * 0.5,
                "mW",
                None,
            ),
            field(
                "g_coil_tx",
                s16_be(p[5], p[6]),
                "raw",
                Some(format!("0x{:02X}{:02X}", p[5], p[6])),
            ),
            field(
                "g_coil_tx_scaled",
                s16_be(p[5], p[6]) as f64 * 0.0001,
                "",
                None,
            ),
        ],
        0x81 if p.len() >= 1 && p[0] == 0xFE => {
            let mut fields = vec![field("selector", "MPP XID", "", Some("0xFE".into()))];
            if p.len() >= 7 {
                fields.extend([
                    field(
                        "restricted",
                        p[1] & 0x80 != 0,
                        "",
                        Some(format!("0x{:02X}", p[1])),
                    ),
                    field(
                        "v_rect",
                        p[3] as u16 * 20,
                        "mV",
                        Some(format!("0x{:02X}", p[3])),
                    ),
                    field(
                        "alpha_0r",
                        s8(p[4]) as f64 / 100.0,
                        "",
                        Some(format!("0x{:02X}", p[4])),
                    ),
                    field(
                        "alpha_1r",
                        s8(p[5]) as f64 / 100.0,
                        "",
                        Some(format!("0x{:02X}", p[5])),
                    ),
                    field(
                        "alpha_k_threshold",
                        s8(p[6]) as i64,
                        "",
                        Some(format!("0x{:02X}", p[6])),
                    ),
                ]);
            }
            fields
        }
        0x81 => {
            let id = p
                .iter()
                .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
            vec![field(
                "extended_device_id",
                format!("0x{id:0width$X}", width = p.len() * 2),
                "",
                None,
            )]
        }
        0x84 if p.len() >= 4 => vec![
            field("reserved_byte_0", p[0], "", Some(format!("0x{:02X}", p[0]))),
            field(
                "minimum_charge_power",
                p[1] & 0x0F,
                "",
                Some(format!("0x{:02X}", p[1])),
            ),
            field(
                "concurrent_stream_count",
                (p[3] >> 5) & 7,
                "",
                Some(format!("0x{:02X}", p[3])),
            ),
            field("data_stream_buffer_size", (p[3] >> 2) & 7, "", None),
        ],
        0x88 if p.len() >= 9 => vec![
            field(
                "p_received",
                u16_be(p[1], p[2]),
                "mW",
                Some(format!("0x{:02X}{:02X}", p[1], p[2])),
            ),
            field(
                "p_rect",
                u16_be(p[3], p[4]),
                "mW",
                Some(format!("0x{:02X}{:02X}", p[3], p[4])),
            ),
            field(
                "v_rect",
                u16_be(p[5], p[6]),
                "mV",
                Some(format!("0x{:02X}{:02X}", p[5], p[6])),
            ),
            field(
                "i_rect",
                u16_be(p[7], p[8]),
                "mA",
                Some(format!("0x{:02X}{:02X}", p[7], p[8])),
            ),
        ],
        0x90 if p.len() >= 10 => vec![
            field(
                "g_coil_t",
                u16_be(p[1], p[2]),
                "",
                Some(format!("0x{:02X}{:02X}", p[1], p[2])),
            ),
            field(
                "alpha_fm_itx",
                s16_be(p[3], p[4]),
                "",
                Some(format!("0x{:02X}{:02X}", p[3], p[4])),
            ),
            field(
                "alpha_fm_irect",
                s16_be(p[5], p[6]),
                "",
                Some(format!("0x{:02X}{:02X}", p[5], p[6])),
            ),
            field(
                "alpha_fm_vrect",
                s16_be(p[8], p[9]),
                "",
                Some(format!("0x{:02X}{:02X}", p[8], p[9])),
            ),
        ],
        0x96 if p.len() >= 10 => {
            let i_rect = u16_be(p[8] & 0x0F, p[9]);
            vec![
                field(
                    "calibration_point_index",
                    p[0] & 0x7F,
                    "",
                    Some(format!("0x{:02X}", p[0])),
                ),
                field("operation", p[1] & 3, "", Some(format!("0x{:02X}", p[1]))),
                field(
                    "operation_desc",
                    match p[1] & 3 {
                        1 => "Clear point",
                        2 => "Capture power parameters",
                        _ => "Reserved",
                    },
                    "",
                    None,
                ),
                field("received_power", u16_be(p[2], p[3]), "mW", None),
                field("p_rect", u16_be(p[4], p[5]), "mW", None),
                field("v_rect", u16_be(p[6], p[7]), "mV", None),
                field(
                    "i_rect",
                    i_rect,
                    "mA",
                    Some(format!("0x{:01X}{:02X}", p[8] & 0x0F, p[9])),
                ),
            ]
        }
        0xA8 if p.len() >= 7 => vec![
            field(
                "g0",
                s16_be(p[1], p[2]) as f64 * 0.001,
                "",
                Some(format!("0x{:02X}{:02X}", p[1], p[2])),
            ),
            field(
                "g1",
                s16_be(p[3], p[4]) as f64 * 0.001,
                "",
                Some(format!("0x{:02X}{:02X}", p[3], p[4])),
            ),
            field(
                "d0",
                s16_be(p[5], p[6]) as f64 * 0.001,
                "",
                Some(format!("0x{:02X}{:02X}", p[5], p[6])),
            ),
        ],
        0x16 | 0x17 | 0x26 | 0x27 | 0x36 | 0x37 | 0x46 | 0x47 | 0x56 | 0x57 | 0x66 | 0x67
        | 0x76 | 0x77 | 0x98 | 0x99 => vec![
            field("packet_type", "ADT", "", None),
            field("payload", hex_bytes(p), "", None),
        ],
        0x1A | 0x1B | 0x2A | 0x2B => vec![
            field("packet_type", "Proprietary", "", None),
            field("payload", hex_bytes(p), "", None),
        ],
        _ => bytes_fields(p),
    }
}

fn decode_fsk_fields(header: u8, p: &[u8]) -> Vec<QiField> {
    if p.is_empty() {
        if let Some(name) = fsk_bare_name(header) {
            let mut fields = vec![field("pattern", name, "", Some(format!("0x{header:02X}")))];
            return fields;
        }
    }
    match header {
        0x1F if !p.is_empty() => {
            let v = p[0];
            let desc = if v <= 100 {
                format!("{v} %")
            } else if v == 0xFE {
                "Battery level temporarily not available".into()
            } else if v == 0xFF {
                "Device does not have a battery".into()
            } else {
                format!("Reserved (0x{v:02X})")
            };
            vec![
                field(
                    "charge_status",
                    v,
                    if v <= 100 { "%" } else { "" },
                    Some(format!("0x{v:02X}")),
                ),
                field("charge_status_desc", desc, "", None),
            ]
        }
        0x23 if p.len() >= 2 => vec![
            field("status", p[0] & 3, "", Some(format!("0x{:02X}", p[0]))),
            field(
                "status_desc",
                match p[0] & 3 {
                    0 => "Success",
                    1 => "Pending",
                    2 => "Fail",
                    _ => "Busy",
                },
                "",
                None,
            ),
            field(
                "error_code",
                p[1] & 0x0F,
                "",
                Some(format!("0x{:02X}", p[1])),
            ),
            field(
                "error_desc",
                if p[1] & 0x0F == 1 {
                    "Not Supported"
                } else {
                    "None / Reserved"
                },
                "",
                None,
            ),
        ],
        0x25 if !p.is_empty() => {
            let mut fields = vec![field("request", p[0], "", Some(format!("0x{:02X}", p[0])))];
            add_remaining(&mut fields, p, 1);
            fields
        }
        0x2E if p.len() >= 2 => vec![
            field("reserved", p[0], "", Some(format!("0x{:02X}", p[0]))),
            field("parameter", p[1], "", Some(format!("0x{:02X}", p[1]))),
            field("request_type", get_name(p[1]), "", None),
        ],
        0x2F if p.len() >= 2 => {
            let mask = u16_be(p[0], p[1]);
            vec![
                field("streams_mask", mask, "", Some(format!("0x{mask:04X}"))),
                field(
                    "enabled_streams",
                    serde_json::json!((0..16)
                        .filter(|bit| mask & (1 << bit) != 0)
                        .collect::<Vec<_>>()),
                    "",
                    None,
                ),
            ]
        }
        0x30 => vec![field("ptx_id", hex_bytes(p), "", None)],
        0x31 if p.len() >= 3 => {
            let guar = p[0] & 0x3F;
            let pot = p[2];
            vec![
                field(
                    "power_class",
                    p[0] >> 6,
                    "",
                    Some(format!("0x{:02X}", p[0])),
                ),
                field("guaranteed_power", guar, "", Some(format!("0x{guar:02X}"))),
                field("guaranteed_power_w", (guar as f64) * 0.5, "W", None),
                field("potential_power", pot, "", Some(format!("0x{pot:02X}"))),
                field("potential_power_w", (pot as f64) * 0.5, "W", None),
            ]
        }
        0x32 if !p.is_empty() => vec![
            field("tps", p[0] & 0x80 != 0, "", Some(format!("0x{:02X}", p[0]))),
            field("tde", p[0] & 0x40 != 0, "", None),
            field("tds", p[0] & 0x20 != 0, "", None),
        ],
        0x34 if p.len() >= 3 => vec![
            field(
                "response",
                if p[0] == 1 { "Accept" } else { "Reject" },
                "",
                Some(format!("0x{:02X}", p[0])),
            ),
            field(
                "reason",
                if p[1] == 3 { "FO_DETECTED" } else { "Reserved" },
                "",
                Some(format!("0x{:02X}", p[1])),
            ),
            field("parameter", p[2], "", Some(format!("0x{:02X}", p[2]))),
        ],
        0x3F if !p.is_empty() => match p[0] {
            0 if p.len() >= 3 => {
                let voltage_raw = u16_be(p[1] & 0x3F, p[2]);
                vec![
                    field("B0_selector", "INV", "", Some("0x00".into())),
                    field(
                        "B1_reserved_b7_b6",
                        p[1] >> 6,
                        "",
                        Some(format!("0x{:X}", p[1] >> 6)),
                    ),
                    field(
                        "B1_inverter_voltage_b13_b8",
                        p[1] & 0x3F,
                        "",
                        Some(format!("0x{:02X}", p[1] & 0x3F)),
                    ),
                    field(
                        "B2_inverter_voltage_b7_b0",
                        p[2],
                        "",
                        Some(format!("0x{:02X}", p[2])),
                    ),
                    field(
                        "inverter_voltage_raw",
                        voltage_raw,
                        "2 mV",
                        Some(format!("0x{voltage_raw:04X}")),
                    ),
                    field("inverter_voltage", voltage_raw as u32 * 2, "mV", None),
                ]
            }
            0x01 if p.len() >= 3 => vec![
                field("B0_selector", "SDSR", "", Some("0x01".into())),
                field(
                    "B1_reserved_b7_b4",
                    p[1] >> 4,
                    "",
                    Some(format!("0x{:X}", p[1] >> 4)),
                ),
                field(
                    "B1_stream_number",
                    p[1] & 0x0F,
                    "",
                    Some(format!("0x{:02X}", p[1])),
                ),
                field(
                    "B2_reserved_b7_b4",
                    p[2] >> 4,
                    "",
                    Some(format!("0x{:X}", p[2] >> 4)),
                ),
                field(
                    "B2_response_type",
                    p[2] & 0x0F,
                    "",
                    Some(format!("0x{:02X}", p[2])),
                ),
                field(
                    "B2_response_type_name",
                    sdsr_type_name(p[2] & 0x0F),
                    "",
                    None,
                ),
            ],
            0x02 if p.len() >= 3 => {
                let k_high_nibble = p[1] & 0x0F;
                let value = u16_be(k_high_nibble, p[2]);
                vec![
                    field("B0_selector", "KEST", "", Some("0x02".into())),
                    field(
                        "B1_reserved_b7_b4",
                        p[1] >> 4,
                        "",
                        Some(format!("0x{:X}", p[1] >> 4)),
                    ),
                    field(
                        "B1_estimated_k_b11_b8",
                        k_high_nibble,
                        "",
                        Some(format!("0x{k_high_nibble:X}")),
                    ),
                    field(
                        "B2_estimated_k_b7_b0",
                        p[2],
                        "",
                        Some(format!("0x{:02X}", p[2])),
                    ),
                    field(
                        "B1_B2_estimated_k_raw",
                        value,
                        "",
                        Some(format!("0x{value:03X}")),
                    ),
                    field("estimated_k", value as f64 / 4095.0, "", None),
                ]
            }
            _ => vec![
                field("B0_selector", p[0], "", Some(format!("0x{:02X}", p[0]))),
                field("data", hex_bytes(&p[1..]), "", None),
            ],
        },
        0x40 if !p.is_empty() => vec![
            field(
                "foreign_object_result",
                p[0] & 7,
                "",
                Some(format!("0x{:02X}", p[0])),
            ),
            field(
                "result_desc",
                match p[0] & 7 {
                    0 => "Cannot compute",
                    1 => "Safe",
                    2 => "Unsafe",
                    3 => "Uncertain",
                    _ => "Reserved",
                },
                "",
                None,
            ),
        ],
        0x43 if p.len() >= 4 => vec![field(
            "capability_mask",
            u32::from_be_bytes([p[0], p[1], p[2], p[3]]),
            "",
            Some(format!(
                "0x{:02X}{:02X}{:02X}{:02X}",
                p[0], p[1], p[2], p[3]
            )),
        )],
        0x4F if p.len() >= 4 => {
            let req = p[0] & 7;
            let parameter = u16_be(p[2], p[3]);
            vec![
                field("request", req, "", Some(format!("0x{:02X}", p[0]))),
                field("request_desc", sadc_name(req), "", None),
                field(
                    "stream_number",
                    p[1] & 0x0F,
                    "",
                    Some(format!("0x{:02X}", p[1])),
                ),
                field(
                    "parameter",
                    parameter,
                    "",
                    Some(format!("0x{parameter:04X}")),
                ),
            ]
        }
        0x54 if p.len() >= 5 => vec![
            field(
                "invalid",
                p[0] & 1 != 0,
                "",
                Some(format!("0x{:02X}", p[0])),
            ),
            field(
                "dploss_alpha",
                u16_be(p[1], p[2]),
                "",
                Some(format!("0x{:02X}{:02X}", p[1], p[2])),
            ),
            field(
                "dploss_beta",
                u16_be(p[3], p[4]),
                "",
                Some(format!("0x{:02X}{:02X}", p[3], p[4])),
            ),
        ],
        0x5A if p.len() >= 2 => vec![
            field("capabilities", p[1], "", Some(format!("0x{:02X}", p[1]))),
            field("cpm", p[1] & 1 != 0, "", None),
            field("npm", p[1] & 2 != 0, "", None),
            field("lpm", p[1] & 4 != 0, "", None),
            field("hpm", p[1] & 8 != 0, "", None),
        ],
        0x5F if p.len() >= 3 => vec![field(
            "g_coil_r",
            s16_be(p[1], p[2]),
            "",
            Some(format!("0x{:02X}{:02X}", p[1], p[2])),
        )],
        0x61 if p.len() >= 6 => vec![
            field("g_npm_c0", u16_be(p[0], p[1]), "", None),
            field("g_hpm_c0", u16_be(p[2], p[3]), "", None),
            field("g_cpm_c0", u16_be(p[4], p[5]), "", None),
        ],
        0x88 if p.len() >= 9 => {
            let coefficient = u16_be(p[1], p[2]);
            vec![
                field(
                    "B0_reserved",
                    p[0],
                    "",
                    Some(format!("0x{:02X}", p[0])),
                ),
                field(
                    "g_coil_rx_pla2",
                    coefficient,
                    "",
                    Some(format!("0x{:04X}", coefficient)),
                ),
                field("g_coil_rx_pla2_expected", 20_000, "", None),
                field(
                    "g_coil_rx_pla2_valid",
                    coefficient == 20_000,
                    "",
                    None,
                ),
                field("B3_B8_reserved", hex_bytes(&p[3..9]), "", None),
            ]
        }
        0x8F if !p.is_empty() => {
            let selector = p[0] >> 4;
            if selector == 0 && p.len() >= 9 {
                let device_id =
                    (u32::from(p[4] & 0x7F) << 13) | (u32::from(p[5]) << 5) | u32::from(p[6] >> 3);
                vec![
                    field("selector", "XID", "", Some(format!("0x{:02X}", p[0]))),
                    field(
                        "app",
                        if p[0] & 2 != 0 {
                            "Active Alignment"
                        } else {
                            "Magnetic Attachment"
                        },
                        "",
                        None,
                    ),
                    field("uid", p[0] & 1 != 0, "", None),
                    field(
                        "device_identifier",
                        device_id,
                        "",
                        Some(format!("0x{device_id:05X}")),
                    ),
                ]
            } else if selector == 1 && p.len() >= 7 {
                let buffer_n = (p[6] >> 2) & 3;
                vec![
                    field("selector", "ECAP", "", Some(format!("0x{:02X}", p[0]))),
                    field(
                        "potential_load_power",
                        p[2] as u16 * 100,
                        "mW",
                        Some(format!("0x{:02X}", p[2])),
                    ),
                    field(
                        "negotiable_load_power",
                        p[4] as u16 * 100,
                        "mW",
                        Some(format!("0x{:02X}", p[4])),
                    ),
                    field(
                        "calibration_supported",
                        (p[5] >> 4) & 3 != 0,
                        "",
                        Some(format!("0x{:02X}", p[5])),
                    ),
                    field("power_limit_reason", p[5] & 0x0F, "", None),
                    field(
                        "buffer_size",
                        16_u16 * (1_u16 << buffer_n),
                        "B",
                        Some(format!("0x{:02X}", p[6])),
                    ),
                    field("concurrent_streams", p[6] & 3, "", None),
                ]
            } else {
                vec![
                    field("selector", selector, "", Some(format!("0x{:02X}", p[0]))),
                    field("data", hex_bytes(&p[1..]), "", None),
                ]
            }
        }
        0xA0 if p.len() >= 12 => {
            let modes = [("cpm", 0), ("lpm", 3), ("npm", 6), ("hpm", 9)];
            modes
                .iter()
                .flat_map(|(name, off)| {
                    [
                        field(
                            &format!("{name}_v_ref0"),
                            p[*off],
                            "",
                            Some(format!("0x{:02X}", p[*off])),
                        ),
                        field(
                            &format!("{name}_v_ref1"),
                            p[*off + 1],
                            "",
                            Some(format!("0x{:02X}", p[*off + 1])),
                        ),
                        field(
                            &format!("{name}_potential_power"),
                            p[*off + 2] as u16 * 100,
                            "mW",
                            Some(format!("0x{:02X}", p[*off + 2])),
                        ),
                    ]
                })
                .collect()
        }
        0x16 | 0x17 | 0x26 | 0x27 | 0x36 | 0x37 | 0x46 | 0x47 | 0x56 | 0x57 | 0x66 | 0x67
        | 0x76 | 0x77 | 0x98 | 0x99 => vec![
            field("packet_type", "ADT", "", None),
            field("payload", hex_bytes(p), "", None),
        ],
        0x1C | 0x1D | 0x2C | 0x2D | 0x3E | 0x4E => vec![
            field("packet_type", "Proprietary", "", None),
            field("payload", hex_bytes(p), "", None),
        ],
        _ => bytes_fields(p),
    }
}

/// Full structured decode of an ASK/FSK log line.
pub fn decode_qi_message(line: &str) -> Option<QiDecode> {
    let raw = {
        let t = line.trim();
        if t.is_empty() {
            return None;
        }
        t.to_string()
    };
    let (hex_str, direction) = extract_hex_region(&raw)?;
    let bytes = parse_hex_bytes(&hex_str);
    if bytes.is_empty() {
        return None;
    }
    // Some capture sources strip the FSK 0x3F header before logging, leaving
    // B0(selector), B1, B2. That collides with real one-byte FSK headers whose
    // payload + XOR checksum also span three bytes (e.g. `FSK 0 0 0` is NULL
    // header 0x00, payload 0x00, checksum 0x00 — not headerless 0x3F INV).
    let headerless_fsk_3f = direction == "FSK"
        && bytes.len() == 3
        && match bytes[0] {
            // 0x00 is NULL/NAK; only treat as headerless 0x3F when the normal
            // one-byte-header XOR checksum cannot validate.
            0x00 => bytes[0] ^ bytes[1] != bytes[2],
            // 0x01/0x02 are 0x3F SDSR/KEST selectors and also valid FSK headers;
            // capture logs without 0x3F use this three-byte shape intentionally.
            0x01 | 0x02 => true,
            _ => false,
        };
    let (header, payload, checksum, checksum_ok) = if headerless_fsk_3f {
        (0x3F, bytes.clone(), None, None)
    } else {
        let header = bytes[0];
        let (payload, checksum, checksum_ok) = split_payload_checksum(header, &bytes[1..]);
        (header, payload, checksum, checksum_ok)
    };

    let (meta, known) = if direction == "ASK" {
        match ask_packet(header) {
            Some(m) => (m, true),
            None => (
                super::defs::PacketMeta {
                    name: "UNK",
                    desc: "Unknown ASK packet",
                    profile: "unknown",
                },
                false,
            ),
        }
    } else {
        match fsk_packet(header) {
            Some(m) => (m, true),
            None => (
                super::defs::PacketMeta {
                    name: "UNK",
                    desc: "Unknown FSK packet",
                    profile: "unknown",
                },
                false,
            ),
        }
    };

    let fields = if direction == "ASK" {
        decode_ask_fields(header, &payload)
    } else {
        decode_fsk_fields(header, &payload)
    };

    let summary = format!(
        "{direction} {} header=0x{header:02X} fields={} known={known}",
        meta.name,
        fields.len()
    );

    Some(QiDecode {
        raw,
        direction: direction.into(),
        header,
        name: meta.name.into(),
        desc: meta.desc.into(),
        profile: meta.profile.into(),
        bytes,
        payload_hex: hex_bytes(&payload),
        payload,
        checksum,
        checksum_ok,
        fields,
        known,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_ask_ept() {
        let d = decode_qi_message("TX0:[12:00:00.000] ASK 02 01 F ").unwrap();
        assert_eq!(d.direction, "ASK");
        assert_eq!(d.name, "EPT");
        assert_eq!(d.header, 0x02);
        assert!(d.known);
        assert!(d.fields.iter().any(|f| f.name == "reason"));
    }

    #[test]
    fn decode_ask_ce_signed() {
        let d = decode_qi_message("ASK 03 FE F ").unwrap();
        assert_eq!(d.name, "CE");
        let ce = d.fields.iter().find(|f| f.name == "control_error").unwrap();
        assert_eq!(ce.value, serde_json::json!(-2));
    }

    #[test]
    fn decode_fsk_3f_uses_payload_b0_as_selector() {
        let d = decode_qi_message("FSK 3F 02 03 FF F ").unwrap();
        assert_eq!(d.header, 0x3F);
        assert_eq!(d.payload, vec![0x02, 0x03, 0xFF]);
        assert!(d
            .fields
            .iter()
            .any(|f| f.name == "B0_selector" && f.value == serde_json::json!("KEST")));
        let raw = d
            .fields
            .iter()
            .find(|f| f.name == "B1_B2_estimated_k_raw")
            .unwrap();
        assert_eq!(raw.value, serde_json::json!(0x3FF));
    }

    #[test]
    fn decode_fsk_3f_sdsr_uses_low_nibbles() {
        let d = decode_qi_message("FSK 3F 01 A5 C3 F ").unwrap();
        assert_eq!(d.payload, vec![0x01, 0xA5, 0xC3]);
        assert!(d
            .fields
            .iter()
            .any(|f| f.name == "B1_stream_number" && f.value == serde_json::json!(0x05)));
        assert!(d
            .fields
            .iter()
            .any(|f| f.name == "B2_response_type_name" && f.value == serde_json::json!("ERR_CRC")));
    }

    #[test]
    fn decode_fsk_3f_inv_assembles_b1_low_six_bits_with_b2() {
        let d = decode_qi_message("FSK 3F 00 15 66 4C F ").unwrap();
        assert_eq!(d.payload, vec![0x00, 0x15, 0x66]);
        assert_eq!(d.checksum, Some(0x4C));
        let raw = d
            .fields
            .iter()
            .find(|f| f.name == "inverter_voltage_raw")
            .unwrap();
        assert_eq!(raw.value, serde_json::json!(0x1566));
        let voltage = d
            .fields
            .iter()
            .find(|f| f.name == "inverter_voltage")
            .unwrap();
        assert_eq!(voltage.value, serde_json::json!(0x1566 * 2));
    }

    #[test]
    fn decode_headerless_fsk_3f_payload() {
        // Capture source omitted the 0x3F header: B0=selector, B1, B2.
        let d = decode_qi_message("FSK 01 03 02 F ").unwrap();
        assert_eq!(d.header, 0x3F);
        assert_eq!(d.payload, vec![0x01, 0x03, 0x02]);
        assert!(d
            .fields
            .iter()
            .any(|f| f.name == "B0_selector" && f.value == serde_json::json!("SDSR")));
    }

    #[test]
    fn decode_fsk_plap_2_coil_coefficient() {
        // B0 reserved, B1:B2 g_COIL,RX,PLA2 = 0x4E20 = 20000,
        // B3..B8 reserved, checksum = XOR(header + payload) = 0xE6.
        let d =
            decode_qi_message("FSK 88 00 4E 20 00 00 00 00 00 00 E6 F").unwrap();
        assert_eq!(d.name, "PLAP_2");
        assert_eq!(d.payload, vec![0x00, 0x4E, 0x20, 0, 0, 0, 0, 0, 0]);
        assert_eq!(d.checksum_ok, Some(true));
        assert!(d.fields.iter().any(|field| {
            field.name == "g_coil_rx_pla2" && field.value == serde_json::json!(20_000)
        }));
        assert!(d.fields.iter().any(|field| {
            field.name == "g_coil_rx_pla2_valid" && field.value == serde_json::json!(true)
        }));
    }

    #[test]
    fn decode_fsk_null_zeros_is_not_headerless_3f() {
        // 0x00 ^ 0x00 = 0x00 — valid NULL/NAK frame, not 0x3F INV payload.
        let d = decode_qi_message("FSK 0 0 0").unwrap();
        assert_eq!(d.header, 0x00);
        assert_ne!(d.header, 0x3F);
        assert_eq!(d.payload, vec![0x00]);
        assert_eq!(d.checksum, Some(0x00));
        assert_eq!(d.checksum_ok, Some(true));
    }
}
