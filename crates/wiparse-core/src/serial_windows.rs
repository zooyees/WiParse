//! Windows Setup/registry helpers for WIPARSE UART COM ports.
//!
//! ICD WIPARSE-IF-001: DAP `MI_00`, UART `MI_01` (optional CDC), I2C `MI_03`.
//! WinUSB interfaces often still have a leftover `PortName` that is **not** a
//! real `usbser` device — `CreateFile(\\.\COMx)` then fails with
//! ERROR_FILE_NOT_FOUND. After unplug, `Enum\USB` keeps those values until
//! CONFIGFLAG_REMOVED is set. Only **present UART** names are used to tag live
//! serialport entries; stale / I2C / DAP names are dropped.

use std::collections::HashSet;
use winreg::enums::HKEY_LOCAL_MACHINE;
use winreg::RegKey;

const USB_ENUM: &str = r"SYSTEM\CurrentControlSet\Enum\USB";
const VIDPID: &str = "VID_C251&PID_F01A";
/// CONFIGFLAG_REMOVED — instance is not currently present.
const CONFIGFLAG_REMOVED: u32 = 0x0000_0020;
const UART_MI: u8 = 1;
const UART_CDC_DATA_MI: u8 = 2;
const DAP_MI: u8 = 0;
const I2C_MI: u8 = 3;

#[derive(Debug, Clone)]
pub struct RegistryCom {
    pub port_name: String,
    pub friendly: String,
    pub mi: Option<u8>,
    pub present: bool,
}

pub fn wiparse_uart_com_ports() -> Vec<RegistryCom> {
    let mut hits: Vec<RegistryCom> = scan_dongle_coms()
        .into_iter()
        .filter(|c| c.present && is_uart_com(c))
        .collect();
    let has_mi1 = hits.iter().any(|c| c.mi == Some(UART_MI));
    if has_mi1 {
        hits.retain(|c| c.mi != Some(UART_CDC_DATA_MI));
    }
    let mut seen = HashSet::new();
    hits.retain(|c| seen.insert(normalize_com(&c.port_name)));
    hits
}

/// COM names on this VID/PID that must not be listed (unplugged or non-UART).
pub fn drop_wiparse_com_names() -> HashSet<String> {
    scan_dongle_coms()
        .into_iter()
        .filter(|c| !c.present || !is_uart_com(c))
        .map(|c| normalize_com(&c.port_name))
        .collect()
}

fn scan_dongle_coms() -> Vec<RegistryCom> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok(usb) = hklm.open_subkey(USB_ENUM) else {
        return Vec::new();
    };

    let keys: Vec<String> = usb.enum_keys().filter_map(|k| k.ok()).collect();
    let mut hits = Vec::new();
    for key in &keys {
        if !key.to_ascii_uppercase().contains(VIDPID) {
            continue;
        }
        let mi = mi_from_hwid(key);
        let Ok(dev) = usb.open_subkey(key) else {
            continue;
        };
        for inst_name in dev.enum_keys().filter_map(|k| k.ok()) {
            let Ok(inst) = dev.open_subkey(&inst_name) else {
                continue;
            };
            let port = inst
                .open_subkey("Device Parameters")
                .ok()
                .and_then(|p| p.get_value::<String, _>("PortName").ok())
                .unwrap_or_default();
            let port = port.trim().to_string();
            if !is_com_name(&port) {
                continue;
            }
            let friendly = inst
                .get_value::<String, _>("FriendlyName")
                .unwrap_or_default();
            let present = instance_present(&inst);
            hits.push(RegistryCom {
                port_name: port,
                friendly,
                mi,
                present,
            });
        }
    }
    hits
}

fn instance_present(inst: &RegKey) -> bool {
    let flags: u32 = inst.get_value("ConfigFlags").unwrap_or(0);
    flags & CONFIGFLAG_REMOVED == 0
}

fn is_uart_com(c: &RegistryCom) -> bool {
    let fu = c.friendly.to_ascii_uppercase();
    if fu.contains("I2C") || fu.contains("CMSIS") || fu.contains("DAP") {
        return false;
    }
    if fu.contains("WIPARSE UART") || (fu.contains("WIPARSE") && fu.contains("UART")) {
        return true;
    }
    match c.mi {
        Some(DAP_MI) | Some(I2C_MI) => false,
        Some(UART_MI) | Some(UART_CDC_DATA_MI) => true,
        _ => false,
    }
}

fn mi_from_hwid(hwid: &str) -> Option<u8> {
    let u = hwid.to_ascii_uppercase();
    let idx = u.find("&MI_")?;
    u.get(idx + 4..idx + 6)
        .and_then(|s| u8::from_str_radix(s, 16).ok())
}

fn is_com_name(name: &str) -> bool {
    let n = normalize_com(name);
    let rest = n.strip_prefix("COM").unwrap_or("");
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

pub fn normalize_com(name: &str) -> String {
    name.trim()
        .trim_start_matches(r"\\.\")
        .to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn com_name_helpers() {
        assert!(is_com_name("COM8"));
        assert!(is_com_name(r"\\.\COM12"));
        assert!(!is_com_name("LPT1"));
        assert!(!is_com_name("COM"));
        assert_eq!(normalize_com(r"\\.\com8"), "COM8");
    }

    #[test]
    fn mi_and_uart_filter() {
        assert_eq!(mi_from_hwid(r"VID_C251&PID_F01A&MI_01"), Some(1));
        assert_eq!(mi_from_hwid(r"VID_C251&PID_F01A&MI_03"), Some(3));
        assert_eq!(mi_from_hwid(r"VID_C251&PID_F01A"), None);
        let uart = RegistryCom {
            port_name: "COM13".into(),
            friendly: "WIPARSE UART".into(),
            mi: Some(1),
            present: true,
        };
        let i2c = RegistryCom {
            port_name: "COM14".into(),
            friendly: "WIPARSE I2C".into(),
            mi: Some(3),
            present: true,
        };
        let ghost = RegistryCom {
            port_name: "COM13".into(),
            friendly: "WIPARSE UART".into(),
            mi: Some(1),
            present: false,
        };
        assert!(is_uart_com(&uart));
        assert!(!is_uart_com(&i2c));
        assert!(is_uart_com(&ghost));
        assert!(!ghost.present);
    }
}
