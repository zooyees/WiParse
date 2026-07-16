//! Protocol constants (ported from Python `protocol_defs.py`).

#[derive(Debug, Clone, Copy)]
pub struct PacketMeta {
    pub name: &'static str,
    pub desc: &'static str,
    pub profile: &'static str,
}

/// Equivalent to C macro `GET_PAYLOAD_LEN(header)`.
pub fn get_payload_len(header: u8) -> u8 {
    let h = header as u16;
    if h <= 0x1F {
        (1 + h / 32) as u8
    } else if h <= 0x7F {
        (2 + (h - 32) / 16) as u8
    } else if h <= 0xDF {
        (8 + (h - 128) / 8) as u8
    } else {
        (20 + (h - 224) / 4) as u8
    }
}

pub fn ask_packet(header: u8) -> Option<PacketMeta> {
    Some(match header {
        0x01 => PacketMeta {
            name: "SS",
            desc: "Signal Strength",
            profile: "bpp",
        },
        0x02 => PacketMeta {
            name: "EPT",
            desc: "End Power Transfer",
            profile: "bpp",
        },
        0x03 => PacketMeta {
            name: "CE",
            desc: "Control Error",
            profile: "bpp",
        },
        0x04 => PacketMeta {
            name: "RP8",
            desc: "Received Power 8-bit",
            profile: "bpp",
        },
        0x05 => PacketMeta {
            name: "CHS",
            desc: "Charge Status",
            profile: "bpp",
        },
        0x06 => PacketMeta {
            name: "PCH",
            desc: "Power Control Hold-off",
            profile: "bpp",
        },
        0x07 => PacketMeta {
            name: "GRQ",
            desc: "General Request",
            profile: "bpp",
        },
        0x09 => PacketMeta {
            name: "NEGO",
            desc: "Renegotiate",
            profile: "bpp",
        },
        0x13 => PacketMeta {
            name: "MSR",
            desc: "Mode Select Request",
            profile: "mpp",
        },
        0x15 => PacketMeta {
            name: "DSR",
            desc: "Data Stream Response",
            profile: "bpp",
        },
        0x18 => PacketMeta {
            name: "CLOAK",
            desc: "Cloak Request",
            profile: "mpp",
        },
        0x19 => PacketMeta {
            name: "XCE",
            desc: "Extended Control Error",
            profile: "mpp",
        },
        0x20 => PacketMeta {
            name: "SRQ",
            desc: "Specific Request",
            profile: "both",
        },
        0x22 => PacketMeta {
            name: "FOD",
            desc: "FOD Status",
            profile: "bpp",
        },
        0x23 => PacketMeta {
            name: "CAL_OP",
            desc: "Calibration Operation",
            profile: "mpp",
        },
        0x25 => PacketMeta {
            name: "ADC",
            desc: "Auxiliary Data Control",
            profile: "bpp",
        },
        0x28 => PacketMeta {
            name: "GET",
            desc: "Get Request",
            profile: "mpp",
        },
        0x29 => PacketMeta {
            name: "EDS",
            desc: "Enabled Data Streams",
            profile: "mpp",
        },
        0x2C => PacketMeta {
            name: "CAL_ENTER",
            desc: "Enter Calibration",
            profile: "mpp",
        },
        0x2D => PacketMeta {
            name: "CAL_EXIT",
            desc: "Exit Calibration",
            profile: "mpp",
        },
        0x31 => PacketMeta {
            name: "RP",
            desc: "Received Power 16-bit",
            profile: "bpp",
        },
        0x38 => PacketMeta {
            name: "SDSR",
            desc: "Simultaneous Data Stream Response",
            profile: "mpp",
        },
        0x48 => PacketMeta {
            name: "SADC",
            desc: "Simultaneous Auxiliary Data Control",
            profile: "mpp",
        },
        0x50 => PacketMeta {
            name: "KEST_COEFF",
            desc: "K-est Coefficients",
            profile: "mpp",
        },
        0x51 => PacketMeta {
            name: "CFG",
            desc: "Configuration",
            profile: "bpp",
        },
        0x54 => PacketMeta {
            name: "WPID/hi",
            desc: "WPID high segment",
            profile: "bpp",
        },
        0x55 => PacketMeta {
            name: "WPID/lo",
            desc: "WPID low segment",
            profile: "bpp",
        },
        0x58 => PacketMeta {
            name: "REPORT/PLA",
            desc: "Report / PLA",
            profile: "mpp",
        },
        0x71 => PacketMeta {
            name: "ID",
            desc: "Identification",
            profile: "bpp",
        },
        0x78 => PacketMeta {
            name: "PLAP",
            desc: "Power Loss Accounting Parameters",
            profile: "mpp",
        },
        0x81 => PacketMeta {
            name: "XID",
            desc: "Extended Identification",
            profile: "both",
        },
        0x84 => PacketMeta {
            name: "ECAP",
            desc: "Extended Capabilities",
            profile: "mpp",
        },
        0x88 => PacketMeta {
            name: "PLA_2",
            desc: "Power Loss Accounting 2",
            profile: "mpp",
        },
        0x90 => PacketMeta {
            name: "PLAP_2",
            desc: "PLA Parameters 2",
            profile: "mpp",
        },
        0x96 => PacketMeta {
            name: "CAL_CAPTURE",
            desc: "Calibration Capture",
            profile: "mpp",
        },
        0xA8 => PacketMeta {
            name: "MATEDQ_COEFF",
            desc: "Mated-Q Coefficients",
            profile: "mpp",
        },
        // ADT / PROP
        0x16 => PacketMeta {
            name: "ADT/16",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x17 => PacketMeta {
            name: "ADT/17",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x1A => PacketMeta {
            name: "PROP/1A",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x1B => PacketMeta {
            name: "PROP/1B",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x26 => PacketMeta {
            name: "ADT/26",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x27 => PacketMeta {
            name: "ADT/27",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x2A => PacketMeta {
            name: "PROP/2A",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x2B => PacketMeta {
            name: "PROP/2B",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x36 => PacketMeta {
            name: "ADT/36",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x37 => PacketMeta {
            name: "ADT/37",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x46 => PacketMeta {
            name: "ADT/46",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x47 => PacketMeta {
            name: "ADT/47",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x56 => PacketMeta {
            name: "ADT/56",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x57 => PacketMeta {
            name: "ADT/57",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x66 => PacketMeta {
            name: "ADT/66",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x67 => PacketMeta {
            name: "ADT/67",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x76 => PacketMeta {
            name: "ADT/76",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x77 => PacketMeta {
            name: "ADT/77",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x98 => PacketMeta {
            name: "ADT/98",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x99 => PacketMeta {
            name: "ADT/99",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        _ => return None,
    })
}

pub fn fsk_packet(header: u8) -> Option<PacketMeta> {
    if let Some(name) = fsk_bare_name(header) {
        return Some(PacketMeta {
            name,
            desc: "FSK bare pattern",
            profile: "fsk_pattern",
        });
    }
    Some(match header {
        0x00 => PacketMeta {
            name: "NULL",
            desc: "Data Not Available",
            profile: "bpp",
        },
        0x01 => PacketMeta {
            name: "ERR",
            desc: "Error Status",
            profile: "mpp",
        },
        0x0A => PacketMeta {
            name: "EPTR",
            desc: "End Power Transfer Request",
            profile: "mpp",
        },
        0x11 => PacketMeta {
            name: "FAST-ACK",
            desc: "MPP Fast ACK",
            profile: "mpp",
        },
        0x13 => PacketMeta {
            name: "MSN",
            desc: "Mode Selection Notification",
            profile: "mpp",
        },
        0x14 => PacketMeta {
            name: "CAL_CAPTURE_RSP",
            desc: "Calibration Capture Response",
            profile: "mpp",
        },
        0x15 => PacketMeta {
            name: "DSR",
            desc: "Data Stream Response",
            profile: "bpp",
        },
        0x1B => PacketMeta {
            name: "CAL_OP_RSP",
            desc: "Calibration Operation Response",
            profile: "mpp",
        },
        0x1E => PacketMeta {
            name: "CLOAK/RCS",
            desc: "Cloak Response / Regulation Control Status",
            profile: "mpp",
        },
        0x1F => PacketMeta {
            name: "CHS",
            desc: "Charge Status",
            profile: "mpp",
        },
        0x23 => PacketMeta {
            name: "MSS",
            desc: "Mode Select Status",
            profile: "mpp",
        },
        0x25 => PacketMeta {
            name: "ADC",
            desc: "Auxiliary Data Control",
            profile: "bpp",
        },
        0x2E => PacketMeta {
            name: "GET",
            desc: "Get Request",
            profile: "mpp",
        },
        0x2F => PacketMeta {
            name: "EDS",
            desc: "Enabled Data Streams",
            profile: "mpp",
        },
        0x30 => PacketMeta {
            name: "PTx_ID",
            desc: "Power Transmitter ID",
            profile: "bpp",
        },
        0x31 => PacketMeta {
            name: "CAP",
            desc: "Capabilities",
            profile: "bpp",
        },
        0x32 => PacketMeta {
            name: "XCAP",
            desc: "Extended Capabilities",
            profile: "bpp",
        },
        0x34 => PacketMeta {
            name: "CAL_ENTER_RSP",
            desc: "Enter Calibration Response",
            profile: "mpp",
        },
        0x3F => PacketMeta {
            name: "INV/SDSR/KEST",
            desc: "Inverter Voltage / SDSR / K-est",
            profile: "mpp",
        },
        0x40 => PacketMeta {
            name: "MATEDQ_RES",
            desc: "Mated-Q Results",
            profile: "mpp",
        },
        0x43 => PacketMeta {
            name: "CAL_CAP",
            desc: "Calibration Capabilities",
            profile: "mpp",
        },
        0x4F => PacketMeta {
            name: "SADC",
            desc: "Simultaneous Auxiliary Data Control",
            profile: "mpp",
        },
        0x54 => PacketMeta {
            name: "dPCAL_PARAM",
            desc: "Calibration Parameter",
            profile: "mpp",
        },
        0x5A => PacketMeta {
            name: "MODECAP",
            desc: "Power Modes Capabilities",
            profile: "mpp",
        },
        0x5F => PacketMeta {
            name: "PLAP",
            desc: "PLA Parameters",
            profile: "mpp",
        },
        0x61 => PacketMeta {
            name: "GMP",
            desc: "Gain Measurement Parameters",
            profile: "mpp",
        },
        0x88 => PacketMeta {
            name: "PLAP_2",
            desc: "Power Loss Accounting Parameters 2",
            profile: "mpp",
        },
        0x8F => PacketMeta {
            name: "XID/ECAP",
            desc: "Extended PTx ID / Extended Capabilities",
            profile: "mpp",
        },
        0xA0 => PacketMeta {
            name: "MODEXCAP",
            desc: "Extended Power Modes Capabilities",
            profile: "mpp",
        },
        0x1C => PacketMeta {
            name: "PROP/1C",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x1D => PacketMeta {
            name: "PROP/1D",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x26 => PacketMeta {
            name: "ADT/26",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x27 => PacketMeta {
            name: "ADT/27",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x2C => PacketMeta {
            name: "PROP/2C",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x2D => PacketMeta {
            name: "PROP/2D",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x36 => PacketMeta {
            name: "ADT/36",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x37 => PacketMeta {
            name: "ADT/37",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x3E => PacketMeta {
            name: "PROP/3E",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x46 => PacketMeta {
            name: "ADT/46",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x47 => PacketMeta {
            name: "ADT/47",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x4E => PacketMeta {
            name: "PROP/4E",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x56 => PacketMeta {
            name: "ADT/56",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x57 => PacketMeta {
            name: "ADT/57",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x66 => PacketMeta {
            name: "ADT/66",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x67 => PacketMeta {
            name: "ADT/67",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x76 => PacketMeta {
            name: "ADT/76",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        0x77 => PacketMeta {
            name: "ADT/77",
            desc: "Variable-length packet",
            profile: "dynamic",
        },
        _ => return None,
    })
}

pub fn fsk_bare_name(byte: u8) -> Option<&'static str> {
    match byte {
        0xFF => Some("ACK"),
        0x33 => Some("ATN"),
        0xAA => Some("ND"),
        0x0F => Some("ATN"),
        // BPP ACK / Qi2.2 ND share 0x55 — report dual label
        0x55 => Some("ACK/ND"),
        0x00 => Some("NAK"),
        _ => None,
    }
}

/// BPP bare FSK patterns (name, description) — Python `BPP_FSK_PATTERNS`.
pub fn bpp_fsk_pattern(byte: u8) -> Option<(&'static str, &'static str)> {
    match byte {
        0x55 => Some(("ACK", "Acknowledge (01010101)")),
        0x00 => Some(("NAK", "Negative acknowledge (00000000)")),
        0xAA => Some(("ND", "Not defined / unsupported (10101010)")),
        0x0F => Some(("ATN", "Attention / request communication (00001111)")),
        _ => None,
    }
}

/// Qi 2.2.1 bare FSK patterns — Python `QI22_FSK_PATTERNS`.
pub fn qi22_fsk_pattern(byte: u8) -> Option<(&'static str, &'static str)> {
    match byte {
        0xFF => Some(("ACK", "Acknowledge (Qi 2.2.1)")),
        0x00 => Some(("NAK", "Negative acknowledge")),
        0x55 => Some(("ND", "Not defined (Qi 2.2.1)")),
        0x33 => Some(("ATN", "Attention (Qi 2.2.1)")),
        _ => None,
    }
}

pub fn ept_reason(code: u8) -> Option<&'static str> {
    match code {
        0x00 => Some("EPT/nul — Unknown"),
        0x01 => Some("EPT/cc — Charge complete"),
        0x02 => Some("EPT/if — Internal fault"),
        0x03 => Some("EPT/ot — Over temperature"),
        0x04 => Some("EPT/ov — Over voltage"),
        0x05 => Some("EPT/oc — Over current"),
        0x06 => Some("EPT/bf — Battery fault"),
        0x08 => Some("EPT/nr — No response"),
        0x0A => Some("EPT/an — Negotiation failed"),
        0x0B => Some("EPT/rst — Request restart"),
        0x0C => Some("EPT/rep — Re-ping"),
        0x0D => Some("EPT/nfc — NFC"),
        _ => None,
    }
}

pub fn bpp_grq_request(code: u8) -> Option<&'static str> {
    match code {
        0x31 => Some("PTx Capabilities (CAP)"),
        0x32 => Some("PTx Extended Capabilities (XCAP)"),
        _ => None,
    }
}

pub fn mpp_grq_request(code: u8) -> Option<&'static str> {
    match code {
        0x00 => Some("PTx Identification"),
        0x01 => Some("PTx Capabilities"),
        0x02 => Some("PTx Extended Identification"),
        0x03 => Some("PTx Extended Capabilities"),
        0x20 => Some("MPP Inverter Voltage (INV)"),
        0x30 => Some("MPP Power Modes Capabilities (MODECAP)"),
        0x31 => Some("MPP Extended Power Modes Capabilities (MODEXCAP)"),
        _ => None,
    }
}

pub fn rp_mode_label(mode: u8) -> &'static str {
    match mode & 0x07 {
        0 => "Default",
        1 => "In-band",
        2 => "Out-of-band",
        _ => "Unknown",
    }
}

pub fn fsk_depth_label(depth: u8) -> &'static str {
    match depth & 0x03 {
        0 => "Depth 0",
        1 => "Depth 1",
        2 => "Depth 2",
        3 => "Depth 3",
        _ => "Unknown",
    }
}

pub fn msr_pref(v: u8) -> &'static str {
    match v & 0x03 {
        0 => "No preference",
        1 => "Keep contract",
        2 => "Do not keep contract",
        _ => "Reserved",
    }
}

pub fn msr_main_mode(v: u8) -> &'static str {
    match v & 0x03 {
        0 => "CPM (Continuous Power Mode)",
        1 => "NPM (Nominal Power Mode)",
        2 => "LPM (Low Power Mode)",
        3 => "HPM (High Power Mode)",
        _ => "Unknown",
    }
}

pub fn fod_type_label(v: u8) -> &'static str {
    match v {
        0 => "FOD/qf — Reference Q-Factor",
        1 => "FOD/rf — Reference Resonance Frequency",
        _ => "Reserved",
    }
}

pub fn srq_type_name(code: u8) -> String {
    match code {
        0x00 => "SRQ/en — End negotiation".into(),
        0x01 => "SRQ/gp — Guaranteed power".into(),
        0x03 => "SRQ/fsk — FSK configuration".into(),
        0x05 => "SRQ/reping — Re-ping".into(),
        0xA0 => "SRQ/pla — PLA format".into(),
        0xA1 => "SRQ/xceMethod — Control error method".into(),
        0xA7 => "SRQ/verSel — Version select".into(),
        0xA9 => "SRQ/xceGain — Control gain".into(),
        0xF0 => "SRQ/freqsel — Frequency select".into(),
        0xF3 => "SRQ/egpl — Power level".into(),
        0xF5 => "SRQ/cloakl — Cloak ping delay (low byte)".into(),
        0xF6 => "SRQ/pcp — Power control profile".into(),
        0xF7 => "SRQ/cloakh — Cloak ping delay (high byte)".into(),
        0xF8 => "SRQ/detect — Cloak detect ping delay".into(),
        c if (0xE0..=0xEF).contains(&c) => {
            format!("SRQ/MppProp — Proprietary parameter (0x{c:02X})")
        }
        c => format!("Request 0x{c:02X}"),
    }
}

pub fn prmc_vendor(code: u16) -> Option<&'static str> {
    Some(match code {
        0x0000 => "None",
        0x0010 => "Texas Instruments",
        0x0014 => "Samsung",
        0x0022 => "LG Electronics",
        0x0025 => "Panasonic",
        0x0029 => "Sony",
        0x0039 => "MediaTek",
        0x0042 => "Samsung Electronics",
        0x0050 => "IDT",
        0x005A => "Apple",
        0x0060 => "Huawei",
        0x006E => "Xiaomi",
        0x0072 => "Google",
        0x008E => "Anker Innovations",
        0x00A5 => "Apple",
        0x00A6 => "Ugreen Group",
        0x00CD => "OnePlus",
        0x00D7 => "OPPO",
        0x00DB => "Vivo",
        0x0159 => "Google Pixel",
        _ => return None,
    })
}

pub fn ask_packet_name(header: u8) -> Option<&'static str> {
    ask_packet(header).map(|m| m.name)
}

pub fn fsk_packet_name(header: u8) -> Option<&'static str> {
    fsk_packet(header).map(|m| m.name)
}

pub fn is_known_ask_header(h: u8) -> bool {
    ask_packet(h).is_some()
}

pub fn is_known_fsk_header(h: u8) -> bool {
    fsk_packet(h).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_len_table() {
        assert_eq!(get_payload_len(0x02), 1);
        assert_eq!(get_payload_len(0x1F), 1);
        assert_eq!(get_payload_len(0x20), 2);
    }

    #[test]
    fn ask_ept_registered() {
        assert_eq!(ask_packet_name(0x02), Some("EPT"));
        assert_eq!(ask_packet_name(0x03), Some("CE"));
    }
}
