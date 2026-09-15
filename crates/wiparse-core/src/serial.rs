//! Serial port helpers.

use crate::dongle::{discover_wifi_uart, uart_tcp_baud_preamble};
use crate::metrics::{parse_metric_frame, MetricSample};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;

#[cfg(windows)]
#[path = "serial_windows.rs"]
mod serial_windows;
#[path = "serial_winusb.rs"]
mod serial_winusb;
pub use serial_winusb::{open_winusb_i2c, open_winusb_uart, WinUsbI2c, WinUsbUart, WINUSB_UART_PREFIX};

const WIFI_PROBE_TIMEOUT: Duration = Duration::from_millis(400);
const WIFI_PROBE_TIMEOUT_FORCE: Duration = Duration::from_millis(900);
const WIFI_CACHE_TTL: Duration = Duration::from_secs(4);
const WIFI_NEG_TTL: Duration = Duration::from_millis(800);

struct WifiUartCache {
    at: Option<Instant>,
    port: Option<PortInfo>,
    probing: bool,
}

static WIFI_UART_CACHE: Mutex<WifiUartCache> = Mutex::new(WifiUartCache {
    at: None,
    port: None,
    probing: false,
});

#[derive(Debug, Error)]
pub enum SerialError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Port(#[from] serialport::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortInfo {
    pub device: String,
    pub description: String,
    pub hwid: String,
    /// `usb` / `wifi` / empty (plain COM).
    #[serde(default)]
    pub transport: String,
    /// Combo-box text; falls back to `device` when empty.
    #[serde(default)]
    pub label: String,
}

impl PortInfo {
    pub fn display_label(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.device
        } else {
            &self.label
        }
    }

    /// Closed combo text: COM / transport only, never the long product string.
    pub fn compact_label(&self) -> String {
        if serial_winusb::parse_winusb_uart_device(&self.device).is_some() {
            return "WIPARSE UART · USB".into();
        }
        match self.transport.as_str() {
            "wifi" => {
                if let Some((host, _)) = parse_tcp_uart_device(&self.device) {
                    format!("WiFi · {host}")
                } else {
                    "WIPARSE UART · WiFi".into()
                }
            }
            "usb" if is_wiparse_labeled(self) => {
                format!("{} · USB", normalize_com_display(&self.device))
            }
            _ => {
                if let Some((host, _)) = parse_tcp_uart_device(&self.device) {
                    format!("WiFi · {host}")
                } else {
                    normalize_com_display(&self.device)
                }
            }
        }
    }

    pub fn from_device(device: impl Into<String>) -> Self {
        let device = device.into();
        if serial_winusb::parse_winusb_uart_device(&device).is_some() {
            return PortInfo {
                label: "WIPARSE UART · USB".into(),
                description: "WIPARSE UART".into(),
                hwid: "USB VID:PID=C251:F01A MI_01".into(),
                transport: "usb".into(),
                device,
            };
        }
        if parse_tcp_uart_device(&device).is_some() {
            let label = if let Some((host, port)) = parse_tcp_uart_device(&device) {
                format!("WIPARSE UART · WiFi · {host}:{port}")
            } else {
                device.clone()
            };
            return PortInfo {
                label,
                description: "WIPARSE UART".into(),
                hwid: "WIFI".into(),
                transport: "wifi".into(),
                device,
            };
        }
        Self {
            description: String::new(),
            hwid: String::new(),
            transport: String::new(),
            label: device.clone(),
            device,
        }
    }
}

fn is_wiparse_labeled(p: &PortInfo) -> bool {
    let blob = format!("{} {} {}", p.label, p.description, p.hwid).to_ascii_uppercase();
    blob.contains("WIPARSE")
}

fn normalize_com_display(device: &str) -> String {
    device.trim().trim_start_matches(r"\\.\").to_string()
}

fn normalize_com_key(device: &str) -> String {
    normalize_com_display(device).to_ascii_uppercase()
}

pub fn list_ports() -> Result<Vec<PortInfo>, SerialError> {
    list_ports_inner(false)
}

/// Re-probe WiFi UART even if the short cache is still warm (Refresh button).
pub fn refresh_ports() -> Result<Vec<PortInfo>, SerialError> {
    list_ports_inner(true)
}

/// Background WiFi TCP probe still running. GUI should repaint and merge `list_ports`.
pub fn wifi_uart_discovery_pending() -> bool {
    WIFI_UART_CACHE
        .lock()
        .ok()
        .map(|c| c.probing)
        .unwrap_or(false)
}

/// Start a background WiFi UART probe if the cache is stale. Does not enumerate COM ports.
pub fn kick_wifi_uart_discovery() {
    let _ = wifi_uart_port(false);
}

fn list_ports_inner(force_wifi: bool) -> Result<Vec<PortInfo>, SerialError> {
    let mut out = Vec::new();
    for p in serialport::available_ports()? {
        out.push(port_from_available(&p));
    }
    overlay_wiparse_usb(&mut out);
    for usb in serial_winusb::list_winusb_uart_ports() {
        if !out.iter().any(|p| p.device == usb.device) {
            out.push(usb);
        }
    }
    if let Some(wifi) = wifi_uart_port(force_wifi) {
        if !out.iter().any(|p| p.device == wifi.device) {
            out.push(wifi);
        }
    }
    out.sort_by(|a, b| {
        transport_rank(&a.transport)
            .cmp(&transport_rank(&b.transport))
            .then_with(|| a.device.to_ascii_lowercase().cmp(&b.device.to_ascii_lowercase()))
    });
    Ok(out)
}

fn overlay_wiparse_usb(out: &mut Vec<PortInfo>) {
    let extra = registry_wiparse_coms();
    let drop = registry_drop_coms();
    out.retain(|p| !drop.contains(&normalize_com_key(&p.device)));
    for p in out.iter_mut() {
        if port_is_wiparse_uart(p, &extra) {
            tag_wiparse_usb(p);
        }
    }
    // Never invent COMs from leftover WinUSB PortName values — those fail
    // CreateFile with ERROR_FILE_NOT_FOUND and survive unplug in Enum\USB.
}

fn registry_drop_coms() -> std::collections::HashSet<String> {
    #[cfg(windows)]
    {
        serial_windows::drop_wiparse_com_names()
    }
    #[cfg(not(windows))]
    {
        std::collections::HashSet::new()
    }
}

struct RegistryCom {
    port_name: String,
}

fn registry_wiparse_coms() -> Vec<RegistryCom> {
    #[cfg(windows)]
    {
        serial_windows::wiparse_uart_com_ports()
            .into_iter()
            .map(|c| RegistryCom {
                port_name: c.port_name,
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn port_is_wiparse_uart(p: &PortInfo, extra: &[RegistryCom]) -> bool {
    if p.transport == "wifi" {
        return false;
    }
    if extra
        .iter()
        .any(|c| normalize_com_key(&c.port_name) == normalize_com_key(&p.device))
    {
        return true;
    }
    let blob = format!("{} {} {}", p.description, p.hwid, p.label).to_ascii_uppercase();
    if blob.contains("I2C") || blob.contains("CMSIS-DAP") {
        return false;
    }
    blob.contains("WIPARSE UART") || (blob.contains("WIPARSE") && blob.contains("UART"))
}

fn tag_wiparse_usb(p: &mut PortInfo) {
    let name = normalize_com_display(&p.device);
    p.transport = "usb".into();
    if p.description.trim().is_empty() || !p.description.to_ascii_uppercase().contains("WIPARSE") {
        p.description = "WIPARSE UART".into();
    }
    if p.hwid.trim().is_empty() {
        p.hwid = "USB VID:PID=C251:F01A".into();
    }
    p.label = format!("{name} · WIPARSE UART · USB");
}

fn transport_rank(t: &str) -> u8 {
    match t {
        "usb" => 0,
        "wifi" => 1,
        _ => 2,
    }
}

fn port_from_available(p: &serialport::SerialPortInfo) -> PortInfo {
    match &p.port_type {
        serialport::SerialPortType::UsbPort(info) => {
            let product = info.product.clone().unwrap_or_default();
            let manufacturer = info.manufacturer.clone().unwrap_or_default();
            let iface = info.interface;
            let wiparse =
                is_wiparse_uart_usb(info.vid, info.pid, &product, &manufacturer, iface);
            let description = if product.is_empty() {
                if wiparse {
                    "WIPARSE UART".into()
                } else {
                    "USB Serial".into()
                }
            } else {
                product.clone()
            };
            let hwid = format!("USB VID:PID={:04X}:{:04X}", info.vid, info.pid);
            if wiparse {
                let name = normalize_com_display(&p.port_name);
                PortInfo {
                    label: format!("{name} · WIPARSE UART · USB"),
                    device: p.port_name.clone(),
                    description,
                    hwid,
                    transport: "usb".into(),
                }
            } else {
                PortInfo {
                    label: if description.is_empty() {
                        p.port_name.clone()
                    } else {
                        format!("{} · {}", p.port_name, description)
                    },
                    device: p.port_name.clone(),
                    description,
                    hwid,
                    transport: "usb".into(),
                }
            }
        }
        _ => {
            let name = p.port_name.clone();
            let blob = name.to_ascii_uppercase();
            if blob.contains("WIPARSE") {
                PortInfo {
                    label: format!("{name} · WIPARSE UART · WiFi"),
                    device: name.clone(),
                    description: "WIPARSE UART".into(),
                    hwid: "WIFI".into(),
                    transport: "wifi".into(),
                }
            } else {
                PortInfo {
                    device: name.clone(),
                    description: String::new(),
                    hwid: String::new(),
                    transport: String::new(),
                    label: name,
                }
            }
        }
    }
}

pub fn is_wiparse_uart_usb(
    vid: u16,
    pid: u16,
    product: &str,
    manufacturer: &str,
    interface: Option<u8>,
) -> bool {
    let blob = format!("{product} {manufacturer}").to_ascii_uppercase();
    if blob.contains("I2C") || blob.contains("CMSIS-DAP") || blob.contains("CMSIS DAP") {
        return false;
    }
    if blob.contains("WIPARSE UART") || (blob.contains("WIPARSE") && blob.contains("UART")) {
        return true;
    }
    // Dongle VID/PID is shared by DAP / UART / I2C. Only MI_01 (CDC data MI_02)
    // is UART; I2C is MI_03.
    if vid == 0xC251 && pid == 0xF01A {
        return matches!(interface, Some(1) | Some(2));
    }
    false
}

fn wifi_port_info(host: &str, tcp_port: u16) -> PortInfo {
    let device = format!("tcp://{host}:{tcp_port}");
    PortInfo {
        label: format!("WIPARSE UART · WiFi · {host}:{tcp_port}"),
        description: "WIPARSE UART".into(),
        hwid: format!("WIFI {host}:{tcp_port}"),
        transport: "wifi".into(),
        device,
    }
}

fn wifi_uart_port(force: bool) -> Option<PortInfo> {
    if force {
        return probe_wifi_uart_sync(WIFI_PROBE_TIMEOUT_FORCE);
    }

    let mut cache = WIFI_UART_CACHE.lock().ok()?;
    if let Some(at) = cache.at {
        if cache.port.is_some() && at.elapsed() < WIFI_CACHE_TTL {
            return cache.port.clone();
        }
        if cache.port.is_none() && at.elapsed() < WIFI_NEG_TTL && !cache.probing {
            return None;
        }
    }
    if cache.probing {
        return cache.port.clone();
    }
    cache.probing = true;
    drop(cache);

    let usb = crate::usb::wiparse_dongle_connected();
    let timeout = if usb {
        WIFI_PROBE_TIMEOUT_FORCE
    } else {
        WIFI_PROBE_TIMEOUT
    };
    let _ = std::thread::Builder::new()
        .name("wiparse-uart-wifi".into())
        .spawn(move || {
            let found = discover_wifi_uart(timeout).map(|(host, port)| wifi_port_info(&host, port));
            if let Ok(mut c) = WIFI_UART_CACHE.lock() {
                c.at = Some(Instant::now());
                c.port = found;
                c.probing = false;
            }
        });

    WIFI_UART_CACHE.lock().ok().and_then(|c| c.port.clone())
}

fn probe_wifi_uart_sync(timeout: Duration) -> Option<PortInfo> {
    if let Ok(mut cache) = WIFI_UART_CACHE.lock() {
        cache.probing = true;
    }
    let found = discover_wifi_uart(timeout).map(|(host, port)| wifi_port_info(&host, port));
    if let Ok(mut cache) = WIFI_UART_CACHE.lock() {
        cache.at = Some(Instant::now());
        cache.port = found.clone();
        cache.probing = false;
    }
    found
}

/// `tcp://host:port` (IPv4 / hostname). Used by the serial monitor for WiFi UART.
pub fn parse_tcp_uart_device(device: &str) -> Option<(String, u16)> {
    let raw = device.trim();
    let rest = raw
        .strip_prefix("tcp://")
        .or_else(|| raw.strip_prefix("TCP://"))?;
    let (host, port_s) = rest.rsplit_once(':')?;
    let host = host.trim().trim_matches(['[', ']']);
    if host.is_empty() {
        return None;
    }
    let port: u16 = port_s.trim().parse().ok()?;
    if port == 0 {
        return None;
    }
    Some((host.to_owned(), port))
}

enum PortBackend {
    Serial(Box<dyn serialport::SerialPort>),
    Tcp(TcpStream),
    WinUsb(serial_winusb::WinUsbUart),
}

impl Read for PortBackend {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Serial(p) => p.read(buf),
            Self::Tcp(s) => s.read(buf),
            Self::WinUsb(p) => p.read(buf),
        }
    }
}

impl Write for PortBackend {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Serial(p) => p.write(buf),
            Self::Tcp(s) => s.write(buf),
            Self::WinUsb(p) => p.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Serial(p) => p.flush(),
            Self::Tcp(s) => s.flush(),
            Self::WinUsb(p) => p.flush(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CapturedEvent {
    Metrics(MetricSample),
    Log {
        line: String,
        qi: Option<crate::protocol::QiLineParse>,
    },
}

pub struct SerialSession {
    port: PortBackend,
    buf: Vec<u8>,
    /// Actual transport opened (may be TCP after WinUSB fallback).
    pub device: String,
}

impl SerialSession {
    pub fn open(device: &str, baud: u32) -> Result<Self, SerialError> {
        if let Some((host, port)) = parse_tcp_uart_device(device) {
            return open_tcp_uart(&host, port, baud, device);
        }
        if let Some(serial) = serial_winusb::parse_winusb_uart_device(device) {
            match serial_winusb::open_winusb_uart(&serial, baud) {
                Ok(uart) => {
                    return Ok(Self {
                        port: PortBackend::WinUsb(uart),
                        buf: Vec::with_capacity(4096),
                        device: device.to_string(),
                    });
                }
                Err(usb_err) => {
                    if let Some(wifi) = probe_wifi_uart_sync(WIFI_PROBE_TIMEOUT_FORCE) {
                        tracing::warn!(
                            "WinUSB UART open failed ({usb_err}); falling back to {}",
                            wifi.device
                        );
                        let mut sess = open_tcp_uart_from_info(&wifi, baud)?;
                        sess.device = wifi.device;
                        return Ok(sess);
                    }
                    return Err(usb_err);
                }
            }
        }
        let port = serialport::new(device, baud)
            .timeout(Duration::from_millis(50))
            .open()?;
        Ok(Self {
            port: PortBackend::Serial(port),
            buf: Vec::with_capacity(4096),
            device: device.to_string(),
        })
    }

    pub fn write_hex(&mut self, hex_data: &str) -> Result<usize, SerialError> {
        let cleaned: String = hex_data.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if cleaned.len() % 2 != 0 {
            return Err(SerialError::Message("hex length must be even".into()));
        }
        let mut bytes = Vec::with_capacity(cleaned.len() / 2);
        let mut i = 0;
        while i < cleaned.len() {
            let b = u8::from_str_radix(&cleaned[i..i + 2], 16)
                .map_err(|e| SerialError::Message(e.to_string()))?;
            bytes.push(b);
            i += 2;
        }
        self.write_bytes(&bytes)
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
        self.port.write_all(bytes)?;
        Ok(bytes.len())
    }

    /// Read available bytes and emit complete lines as events.
    pub fn poll_events(&mut self) -> Result<Vec<CapturedEvent>, SerialError> {
        let mut tmp = [0u8; 2048];
        match self.port.read(&mut tmp) {
            Ok(0) => {}
            Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }

        let mut events = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line_bytes = self.buf.drain(..=pos).collect::<Vec<_>>();
            if line_bytes.last() == Some(&b'\n') {
                line_bytes.pop();
            }
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }
            let line = String::from_utf8_lossy(&line_bytes).to_string();
            if let Some(m) = parse_metric_frame(&line) {
                events.push(CapturedEvent::Metrics(m));
            } else if !line.is_empty() {
                // Qi decode is on-demand in the log hover tooltip (Auto Parse).
                events.push(CapturedEvent::Log { line, qi: None });
            }
        }
        Ok(events)
    }
}

fn open_tcp_uart(host: &str, port: u16, baud: u32, device: &str) -> Result<SerialSession, SerialError> {
    let addr = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| SerialError::Message(format!("no address for {host}:{port}")))?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    if let Some(pkt) = uart_tcp_baud_preamble(baud) {
        let _ = (&stream).write_all(&pkt);
    }
    Ok(SerialSession {
        port: PortBackend::Tcp(stream),
        buf: Vec::with_capacity(4096),
        device: device.to_string(),
    })
}

fn open_tcp_uart_from_info(info: &PortInfo, baud: u32) -> Result<SerialSession, SerialError> {
    let (host, port) = parse_tcp_uart_device(&info.device)
        .ok_or_else(|| SerialError::Message("invalid WiFi UART address".into()))?;
    open_tcp_uart(&host, port, baud, &info.device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tcp_uart_device_ok() {
        let parsed = parse_tcp_uart_device("tcp://192.168.137.123:1234").unwrap();
        assert_eq!(parsed.0, "192.168.137.123");
        assert_eq!(parsed.1, 1234);
        assert_eq!(
            parse_tcp_uart_device("tcp://dap.local:1234").unwrap(),
            ("dap.local".into(), 1234)
        );
        assert!(parse_tcp_uart_device("COM5").is_none());
        assert!(parse_tcp_uart_device("tcp://host").is_none());
    }

    #[test]
    fn wiparse_usb_vid_pid_and_product() {
        assert!(is_wiparse_uart_usb(
            0xC251,
            0xF01A,
            "",
            "WIPARSE",
            Some(1)
        ));
        assert!(is_wiparse_uart_usb(
            0xC251,
            0xF01A,
            "",
            "WIPARSE",
            Some(2)
        ));
        assert!(!is_wiparse_uart_usb(
            0xC251,
            0xF01A,
            "",
            "WIPARSE",
            Some(3)
        ));
        assert!(!is_wiparse_uart_usb(
            0xC251,
            0xF01A,
            "",
            "WIPARSE",
            Some(0)
        ));
        assert!(!is_wiparse_uart_usb(0xC251, 0xF01A, "", "WIPARSE", None));
        assert!(is_wiparse_uart_usb(
            0x1234,
            0x0001,
            "WIPARSE UART",
            "x",
            None
        ));
        assert!(!is_wiparse_uart_usb(
            0xC251,
            0xF01A,
            "WIPARSE I2C",
            "x",
            Some(1)
        ));
        assert!(!is_wiparse_uart_usb(0x0403, 0x6001, "FT232", "FTDI", None));
    }

    #[test]
    fn compact_label_strips_long_product() {
        let long = PortInfo {
            device: r"\\.\COM8".into(),
            description: "USB-Enhanced-SERIAL CH3 (COM8)".into(),
            hwid: "USB VID:PID=1A86:55D3".into(),
            transport: "usb".into(),
            label: "COM8 · USB-Enhanced-SERIAL CH3 (COM8)".into(),
        };
        assert_eq!(long.compact_label(), "COM8");
        assert!(long.display_label().contains("USB-Enhanced-SERIAL"));

        let wiparse = PortInfo {
            device: "COM5".into(),
            description: "WIPARSE UART".into(),
            hwid: "USB VID:PID=C251:F01A".into(),
            transport: "usb".into(),
            label: "COM5 · WIPARSE UART · USB".into(),
        };
        assert_eq!(wiparse.compact_label(), "COM5 · USB");

        let wifi = PortInfo {
            device: "tcp://192.168.137.123:1234".into(),
            description: "WIPARSE UART".into(),
            hwid: "WIFI 192.168.137.123:1234".into(),
            transport: "wifi".into(),
            label: "WIPARSE UART · WiFi · 192.168.137.123:1234".into(),
        };
        assert_eq!(wifi.compact_label(), "WiFi · 192.168.137.123");

        let winusb = PortInfo {
            device: "winusb://wiparse/uart".into(),
            description: "WIPARSE UART".into(),
            hwid: "USB VID:PID=C251:F01A MI_01".into(),
            transport: "usb".into(),
            label: "WIPARSE UART · USB".into(),
        };
        assert_eq!(winusb.compact_label(), "WIPARSE UART · USB");
        assert_eq!(
            PortInfo::from_device("winusb://wiparse/uart").compact_label(),
            "WIPARSE UART · USB"
        );
    }

    #[test]
    fn overlay_tags_sibling_com_from_registry() {
        let extra = [RegistryCom {
            port_name: "COM8".into(),
        }];
        let mut p = PortInfo {
            device: "COM8".into(),
            description: "USB-Enhanced-SERIAL CH3".into(),
            hwid: "USB VID:PID=1A86:55D3".into(),
            transport: "usb".into(),
            label: "COM8 · USB-Enhanced-SERIAL CH3".into(),
        };
        assert!(port_is_wiparse_uart(&p, &extra));
        tag_wiparse_usb(&mut p);
        assert_eq!(p.compact_label(), "COM8 · USB");
        assert!(p.display_label().contains("WIPARSE UART"));

        let i2c = PortInfo {
            device: "COM14".into(),
            description: "WIPARSE I2C".into(),
            hwid: "USB VID:PID=C251:F01A".into(),
            transport: "usb".into(),
            label: "COM14 · WIPARSE I2C".into(),
        };
        assert!(!port_is_wiparse_uart(&i2c, &[]));
    }

    #[test]
    fn display_label_prefers_explicit() {
        let p = PortInfo {
            device: "COM5".into(),
            description: "WIPARSE UART".into(),
            hwid: "USB VID:PID=C251:F01A".into(),
            transport: "usb".into(),
            label: "COM5 · WIPARSE UART · USB".into(),
        };
        assert_eq!(p.display_label(), "COM5 · WIPARSE UART · USB");
        let wifi = PortInfo {
            device: "tcp://192.168.137.123:1234".into(),
            description: "WIPARSE UART".into(),
            hwid: "WIFI 192.168.137.123:1234".into(),
            transport: "wifi".into(),
            label: "WIPARSE UART · WiFi · 192.168.137.123:1234".into(),
        };
        assert_eq!(
            wifi.display_label(),
            "WIPARSE UART · WiFi · 192.168.137.123:1234"
        );
        assert_ne!(p.device, wifi.device);
    }
}
