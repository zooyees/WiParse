//! USB VID/PID classification for debug probes and FT4222 bridges.

use crate::instrument::{Identity, InstrumentKind, ResourceInfo};

const VID_SEGGER: u16 = 0x1366;
const VID_ST: u16 = 0x0483;
const VID_FTDI: u16 = 0x0403;
const PID_FT4222: u16 = 0x601C;

const STLINK_PIDS: &[u16] = &[
    0x3744, 0x3748, 0x374A, 0x374B, 0x374E, 0x374F, 0x3752, 0x3753, 0x3754, 0x3755, 0x3757,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbIface {
    Jlink,
    Stlink,
    CmsisDap,
    Ft4222,
}

impl UsbIface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jlink => "jlink",
            Self::Stlink => "stlink",
            Self::CmsisDap => "cmsis-dap",
            Self::Ft4222 => "ft4222",
        }
    }

    pub fn kind(self) -> InstrumentKind {
        match self {
            Self::Ft4222 => InstrumentKind::UsbBridge,
            _ => InstrumentKind::DebugProbe,
        }
    }

    pub fn manufacturer(self) -> &'static str {
        match self {
            Self::Jlink => "SEGGER",
            Self::Stlink => "STMicroelectronics",
            Self::CmsisDap => "CMSIS-DAP",
            Self::Ft4222 => "FTDI",
        }
    }

    pub fn model(self) -> &'static str {
        match self {
            Self::Jlink => "J-Link",
            Self::Stlink => "ST-Link",
            Self::CmsisDap => "CMSIS-DAP",
            Self::Ft4222 => "FT4222H",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbResource {
    pub iface: UsbIface,
    pub serial: String,
}

impl UsbResource {
    pub fn to_address(&self) -> String {
        let scheme = if self.iface == UsbIface::Ft4222 {
            "bridge"
        } else {
            "probe"
        };
        if self.serial.is_empty() {
            format!("{scheme}://{}", self.iface.as_str())
        } else {
            format!("{scheme}://{}/serial={}", self.iface.as_str(), self.serial)
        }
    }

    pub fn identity(&self) -> Identity {
        Identity {
            manufacturer: self.iface.manufacturer().into(),
            model: self.iface.model().into(),
            serial: self.serial.clone(),
            firmware: String::new(),
            raw: self.to_address(),
        }
    }
}

pub fn parse_usb_resource(address: &str) -> Option<UsbResource> {
    let address = address.trim();
    let (rest, default_iface) = if let Some(rest) = address.strip_prefix("probe://") {
        (rest, None)
    } else if let Some(rest) = address.strip_prefix("bridge://") {
        (rest, Some(UsbIface::Ft4222))
    } else {
        return None;
    };
    let (iface_str, serial) = if let Some((head, tail)) = rest.split_once("/serial=") {
        (head, tail.trim().to_owned())
    } else if let Some((head, tail)) = rest.split_once('?') {
        let serial = tail
            .split('&')
            .find_map(|p| p.strip_prefix("serial="))
            .unwrap_or("")
            .to_owned();
        (head, serial)
    } else {
        (rest, String::new())
    };
    let iface = match iface_str.trim().to_ascii_lowercase().as_str() {
        "jlink" | "j-link" => UsbIface::Jlink,
        "stlink" | "st-link" => UsbIface::Stlink,
        "cmsis-dap" | "dap" | "daplink" => UsbIface::CmsisDap,
        "ft4222" | "ft4222h" | "ftdi4222" => UsbIface::Ft4222,
        "" => default_iface?,
        _ => default_iface?,
    };
    Some(UsbResource { iface, serial })
}

pub fn is_usb_session_address(address: &str) -> bool {
    let a = address.trim();
    a.starts_with("probe://") || a.starts_with("bridge://")
}

pub fn classify_usb(vid: u16, pid: u16, product: &str, manufacturer: &str) -> Option<UsbIface> {
    let blob = format!("{product} {manufacturer}").to_ascii_uppercase();
    if vid == VID_FTDI && pid == PID_FT4222 {
        return Some(UsbIface::Ft4222);
    }
    if vid == VID_SEGGER {
        return Some(UsbIface::Jlink);
    }
    if vid == VID_ST && STLINK_PIDS.contains(&pid) {
        return Some(UsbIface::Stlink);
    }
    if blob.contains("CMSIS-DAP") || blob.contains("CMSIS DAP") || blob.contains("DAPLINK") {
        return Some(UsbIface::CmsisDap);
    }
    None
}

pub fn discover_usb_instruments() -> Vec<ResourceInfo> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    match nusb::list_devices() {
        Ok(iter) => {
            for dev in iter {
                let product = dev.product_string().unwrap_or_default().to_owned();
                let manufacturer = dev.manufacturer_string().unwrap_or_default().to_owned();
                let Some(iface) =
                    classify_usb(dev.vendor_id(), dev.product_id(), &product, &manufacturer)
                else {
                    continue;
                };
                let serial = dev.serial_number().unwrap_or_default().to_owned();
                let spec = UsbResource { iface, serial };
                let address = spec.to_address();
                if !seen.insert(address.clone()) {
                    continue;
                }
                out.push(ResourceInfo {
                    address,
                    transport: "USB".into(),
                    kind: Some(iface.kind()),
                    identity: Some(spec.identity()),
                    probe_error: None,
                });
            }
        }
        Err(error) => {
            tracing::debug!("USB enumerate failed: {error}");
        }
    }
    out.extend(discover_probe_rs_instruments(&seen));
    out
}

fn discover_probe_rs_instruments(seen: &std::collections::HashSet<String>) -> Vec<ResourceInfo> {
    let Ok(lister) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        probe_rs::probe::list::Lister::new().list_all()
    })) else {
        return Vec::new();
    };
    let mut extra = Vec::new();
    for info in lister {
        let ident = info.identifier.to_ascii_lowercase();
        let iface = if ident.contains("j-link") || ident.contains("jlink") {
            UsbIface::Jlink
        } else if ident.contains("st-link") || ident.contains("stlink") {
            UsbIface::Stlink
        } else if ident.contains("cmsis") || ident.contains("dap") {
            UsbIface::CmsisDap
        } else {
            continue;
        };
        let serial = info.serial_number.clone().unwrap_or_default();
        let spec = UsbResource { iface, serial };
        let address = spec.to_address();
        if seen.contains(&address) {
            continue;
        }
        extra.push(ResourceInfo {
            address,
            transport: "USB".into(),
            kind: Some(InstrumentKind::DebugProbe),
            identity: Some(spec.identity()),
            probe_error: None,
        });
    }
    extra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_vid_pid() {
        assert_eq!(
            classify_usb(0x1366, 0x0101, "", ""),
            Some(UsbIface::Jlink)
        );
        assert_eq!(
            classify_usb(0x0483, 0x3748, "", ""),
            Some(UsbIface::Stlink)
        );
        assert_eq!(
            classify_usb(0x0403, 0x601C, "", ""),
            Some(UsbIface::Ft4222)
        );
        assert_eq!(
            classify_usb(0x0D28, 0x0204, "CMSIS-DAP", "ARM"),
            Some(UsbIface::CmsisDap)
        );
    }

    #[test]
    fn parses_resource_urls() {
        let j = parse_usb_resource("probe://jlink/serial=ABC").unwrap();
        assert_eq!(j.iface, UsbIface::Jlink);
        assert_eq!(j.serial, "ABC");
        assert_eq!(j.to_address(), "probe://jlink/serial=ABC");
        let b = parse_usb_resource("bridge://ft4222/serial=FT1").unwrap();
        assert_eq!(b.iface, UsbIface::Ft4222);
        assert_eq!(b.iface.kind(), InstrumentKind::UsbBridge);
    }
}
