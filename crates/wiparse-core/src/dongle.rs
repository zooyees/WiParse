//! WiParse-dongle (ICD WIPARSE-IF-001): CMSIS-DAP + UART + I2C on one board.
//!
//! USB VID/PID `0xC251:0xF01A` enumerates as a composite device. DAP is claimed
//! with probe-rs (WinUSB MI_00) without requiring a target chip; Attach binds a
//! named MCU. UART prefers WinUSB MI_01 then TCP `:1234`. I2C text RPC prefers
//! WinUSB MI_03 then TCP `:1236`.

use crate::instrument::{ControlCommand, Identity, InstrumentError, InstrumentKind};
use crate::probe::{parse_hex_bytes, ProbeSession};
use crate::serial::{open_winusb_i2c, open_winusb_uart, WinUsbI2c, WinUsbUart};
use crate::usb::{parse_usb_resource, UsbIface};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const IF_ID: &str = "WIPARSE-IF-001";
pub const DEFAULT_MDNS: &str = "dap.local";
pub const DEFAULT_IPV4: &str = "192.168.137.123";
pub const UART_TCP_PORT: u16 = 1234;
pub const I2C_TCP_PORT: u16 = 1236;
pub const UART_BAUD_MIN: u32 = 1_200;
pub const UART_BAUD_MAX: u32 = 2_000_000;
pub const UART_BAUD_DEFAULT: u32 = 115_200;
pub const I2C_SPEED_DEFAULT: u32 = 400_000;
pub const I2C_TIMEOUT_DEFAULT_MS: u32 = 50;
const I2C_MAX_FRAME: usize = 256;

#[derive(Debug, Clone)]
pub struct DongleSpec {
    pub serial: String,
    pub host: String,
    pub uart_port: u16,
    pub i2c_port: u16,
    pub uart_com: String,
}

impl Default for DongleSpec {
    fn default() -> Self {
        Self {
            serial: String::new(),
            host: DEFAULT_IPV4.into(),
            uart_port: UART_TCP_PORT,
            i2c_port: I2C_TCP_PORT,
            uart_com: String::new(),
        }
    }
}

pub struct DongleSession {
    pub resource: String,
    pub identity: Identity,
    pub kind: InstrumentKind,
    spec: DongleSpec,
    probe: Option<ProbeSession>,
    i2c: Option<I2cLink>,
    uart: Option<UartLink>,
    uart_baud: u32,
    demo: bool,
    probe_note: String,
}

enum UartLink {
    Tcp(TcpStream),
    Serial(Box<dyn serialport::SerialPort>),
    WinUsb(WinUsbUart),
}

enum I2cLink {
    Tcp(TcpStream),
    WinUsb(WinUsbI2c),
}

impl DongleSession {
    pub fn demo(resource: impl Into<String>) -> Self {
        let resource = resource.into();
        let spec = parse_dongle_resource(&resource).unwrap_or_default();
        let probe = Some(ProbeSession::demo(probe_resource(&spec.serial)));
        Self {
            identity: dongle_identity(&spec, "demo"),
            resource,
            kind: InstrumentKind::WiparseDongle,
            spec,
            probe,
            i2c: None,
            uart: None,
            uart_baud: UART_BAUD_DEFAULT,
            demo: true,
            probe_note: "demo DAP".into(),
        }
    }

    pub fn open(resource: &str) -> Result<Self, InstrumentError> {
        let spec = parse_dongle_or_host(resource).ok_or_else(|| {
            InstrumentError::Io(format!("not a WiParse-dongle resource: {resource}"))
        })?;
        let try_usb = !spec.serial.is_empty()
            || resource.contains("dongle://wiparse")
            || resource.contains("probe://");
        let (probe, probe_note) = if try_usb {
            match ProbeSession::open(&probe_resource(&spec.serial), None) {
                Ok(session) => {
                    let serial = session.identity.serial.clone();
                    let note = if session.identity.firmware.is_empty() {
                        format!("DAP USB claimed serial={serial}")
                    } else {
                        format!(
                            "DAP USB attached serial={serial} chip={}",
                            session.identity.firmware
                        )
                    };
                    (Some(session), note)
                }
                Err(error) => (None, format!("DAP USB not claimed ({error})")),
            }
        } else {
            (None, "DAP USB skipped (TCP-only address)".into())
        };
        Ok(Self {
            identity: dongle_identity(&spec, ""),
            resource: resource.to_owned(),
            kind: InstrumentKind::WiparseDongle,
            spec,
            probe,
            i2c: None,
            uart: None,
            uart_baud: UART_BAUD_DEFAULT,
            demo: false,
            probe_note,
        })
    }

    pub fn execute(&mut self, command: ControlCommand) -> Result<Option<String>, InstrumentError> {
        match command {
            ControlCommand::DongleInfo => self.info(),
            ControlCommand::DongleSetLink {
                host,
                uart_port,
                i2c_port,
            } => self.set_link(host, uart_port, i2c_port),
            ControlCommand::DongleI2cLine { line } => self.i2c_line(&line),
            ControlCommand::DongleI2cScan => self.i2c_line("SCAN"),
            ControlCommand::DongleI2cPing { addr } => {
                self.i2c_line(&format!("PING {}", i2c_addr_token(&addr)?))
            }
            ControlCommand::DongleI2cRecover => self.i2c_line("RECOVER"),
            ControlCommand::DongleI2cSpeed { hz } => match hz {
                Some(hz) => self.i2c_line(&format!("SPEED {hz}")),
                None => self.i2c_line("SPEED"),
            },
            ControlCommand::DongleI2cTimeout { ms } => match ms {
                Some(ms) => self.i2c_line(&format!("TIMEOUT {ms}")),
                None => self.i2c_line("TIMEOUT"),
            },
            ControlCommand::DongleI2cWrite { addr, data_hex } => {
                let mut line = format!("WRITE {}", i2c_addr_token(&addr)?);
                if !data_hex.trim().is_empty() {
                    line.push(' ');
                    line.push_str(&hex_tokens(&parse_hex_bytes(&data_hex)?)?);
                }
                self.i2c_line(&line)
            }
            ControlCommand::DongleI2cRead { addr, len } => {
                let n = (len as usize).clamp(1, I2C_MAX_FRAME);
                self.i2c_line(&format!("READ {} {n}", i2c_addr_token(&addr)?))
            }
            ControlCommand::DongleI2cWriteRead {
                addr,
                data_hex,
                len,
            } => {
                let n = (len as usize).clamp(1, I2C_MAX_FRAME);
                let mut line = format!("WRITEREAD {}", i2c_addr_token(&addr)?);
                if !data_hex.trim().is_empty() {
                    line.push(' ');
                    line.push_str(&hex_tokens(&parse_hex_bytes(&data_hex)?)?);
                }
                line.push(' ');
                line.push_str(&n.to_string());
                self.i2c_line(&line)
            }
            ControlCommand::DongleUartOpen {
                host,
                port,
                baud,
                com,
                send_baud_cmd,
            } => self.uart_open(host, port, baud, com, send_baud_cmd),
            ControlCommand::DongleUartClose => self.uart_close(),
            ControlCommand::DongleUartWrite { data, hex } => self.uart_write(&data, hex),
            ControlCommand::DongleUartRead => self.uart_read(),
            other => {
                let attach = matches!(&other, ControlCommand::ProbeAttach { .. });
                let probe = self.probe.as_mut().ok_or_else(|| {
                    InstrumentError::Io(if self.probe_note.is_empty() {
                        "DAP channel is not attached (plug USB or use Testing Hub TCP USB/IP)"
                            .into()
                    } else {
                        self.probe_note.clone()
                    })
                })?;
                let result = probe.execute(other)?;
                if attach {
                    self.probe_note = if probe.identity.firmware.is_empty() {
                        format!("DAP USB claimed serial={}", probe.identity.serial)
                    } else {
                        format!(
                            "DAP USB attached {} serial={}",
                            probe.identity.firmware, probe.identity.serial
                        )
                    };
                }
                Ok(result)
            }
        }
    }

    fn info(&self) -> Result<Option<String>, InstrumentError> {
        let uart = match &self.uart {
            Some(UartLink::Tcp(_)) => format!(
                "UART tcp {}:{} @{} open",
                self.spec.host, self.spec.uart_port, self.uart_baud
            ),
            Some(UartLink::Serial(_)) => {
                format!("UART {} @{} open", self.spec.uart_com, self.uart_baud)
            }
            Some(UartLink::WinUsb(_)) => format!("UART WinUSB MI_01 @{} open", self.uart_baud),
            None if self.demo => format!(
                "UART tcp {}:{} @{} (demo)",
                self.spec.host, self.spec.uart_port, self.uart_baud
            ),
            None => format!(
                "UART tcp {}:{} idle{}",
                self.spec.host,
                self.spec.uart_port,
                if self.spec.uart_com.is_empty() {
                    String::new()
                } else {
                    format!(" (COM {})", self.spec.uart_com)
                }
            ),
        };
        let i2c = match &self.i2c {
            Some(I2cLink::WinUsb(_)) => "I2C WinUSB MI_03 open".into(),
            Some(I2cLink::Tcp(_)) => {
                format!("I2C tcp {}:{} open", self.spec.host, self.spec.i2c_port)
            }
            None if self.demo => format!("I2C tcp {}:{} demo", self.spec.host, self.spec.i2c_port),
            None => format!(
                "I2C idle (USB MI_03 then tcp {}:{})",
                self.spec.host, self.spec.i2c_port
            ),
        };
        Ok(Some(format!(
            "{IF_ID}  {}\nDAP {}\n{uart}\n{i2c}",
            self.identity.raw, self.probe_note
        )))
    }

    fn set_link(
        &mut self,
        host: String,
        uart_port: u16,
        i2c_port: u16,
    ) -> Result<Option<String>, InstrumentError> {
        let host = host.trim();
        let host_changed = !host.is_empty() && host != self.spec.host;
        let uart_changed = uart_port > 0 && uart_port != self.spec.uart_port;
        let i2c_changed = i2c_port > 0 && i2c_port != self.spec.i2c_port;
        if !host.is_empty() {
            self.spec.host = host.to_owned();
        }
        if uart_port > 0 {
            self.spec.uart_port = uart_port;
        }
        if i2c_port > 0 {
            self.spec.i2c_port = i2c_port;
        }
        // Applying the same settings is idempotent. TCP endpoint changes must
        // not close the other interface or a local COM/WinUSB connection.
        if (host_changed || i2c_changed) && matches!(self.i2c, Some(I2cLink::Tcp(_))) {
            self.i2c = None;
        }
        if (host_changed || uart_changed) && matches!(self.uart, Some(UartLink::Tcp(_))) {
            self.uart = None;
        }
        self.info()
    }

    fn i2c_line(&mut self, line: &str) -> Result<Option<String>, InstrumentError> {
        let line = line.trim();
        if line.is_empty() {
            return Err(InstrumentError::Io("empty I2C command".into()));
        }
        if line.len() > 1023 {
            return Err(InstrumentError::Io("I2C line exceeds 1023 bytes".into()));
        }
        if self.demo {
            return Ok(Some(demo_i2c_reply(line)));
        }
        let stream = self.ensure_i2c()?;
        let mut payload = line.as_bytes().to_vec();
        payload.push(b'\n');
        match stream {
            I2cLink::Tcp(s) => {
                s.write_all(&payload)
                    .map_err(|e| InstrumentError::Io(format!("I2C write: {e}")))?;
                read_line(s, Duration::from_millis(2_000))
            }
            I2cLink::WinUsb(s) => {
                s.write_all(&payload)
                    .map_err(|e| InstrumentError::Io(format!("I2C write: {e}")))?;
                read_line(s, Duration::from_millis(2_500))
            }
        }
        .and_then(|reply| {
            if reply.to_ascii_uppercase().starts_with("ERR") {
                Err(InstrumentError::Io(reply))
            } else {
                Ok(Some(reply))
            }
        })
    }

    fn ensure_i2c(&mut self) -> Result<&mut I2cLink, InstrumentError> {
        if self.i2c.is_none() {
            if let Ok(link) = open_winusb_i2c(&self.spec.serial) {
                self.i2c = Some(I2cLink::WinUsb(link));
            } else {
                let stream =
                    connect_tcp(&self.spec.host, self.spec.i2c_port, Duration::from_secs(3))?;
                let _ = stream.set_read_timeout(Some(Duration::from_millis(2_000)));
                self.i2c = Some(I2cLink::Tcp(stream));
            }
        }
        self.i2c
            .as_mut()
            .ok_or_else(|| InstrumentError::Io("I2C link missing".into()))
    }

    fn uart_open(
        &mut self,
        host: String,
        port: u16,
        baud: u32,
        com: String,
        send_baud_cmd: bool,
    ) -> Result<Option<String>, InstrumentError> {
        let baud = baud.clamp(UART_BAUD_MIN, UART_BAUD_MAX);
        self.uart_baud = baud;
        if !host.trim().is_empty() {
            self.spec.host = host.trim().to_owned();
        }
        if port > 0 {
            self.spec.uart_port = port;
        }
        if !com.trim().is_empty() {
            self.spec.uart_com = com.trim().to_owned();
        }
        self.uart = None;
        if self.demo {
            return Ok(Some(format!(
                "UART demo @{} (tcp {}:{}{})",
                baud,
                self.spec.host,
                self.spec.uart_port,
                if self.spec.uart_com.is_empty() {
                    String::new()
                } else {
                    format!(" / {}", self.spec.uart_com)
                }
            )));
        }
        if !self.spec.uart_com.is_empty() {
            let port = serialport::new(&self.spec.uart_com, baud)
                .timeout(Duration::from_millis(20))
                .open()
                .map_err(|e| InstrumentError::Io(format!("UART {}: {e}", self.spec.uart_com)))?;
            self.uart = Some(UartLink::Serial(port));
            return Ok(Some(format!("UART {} @{} open", self.spec.uart_com, baud)));
        }
        if let Ok(uart) = open_winusb_uart(&self.spec.serial, baud) {
            self.uart = Some(UartLink::WinUsb(uart));
            return Ok(Some(format!("UART WinUSB MI_01 @{baud} open")));
        }
        let stream = connect_tcp(&self.spec.host, self.spec.uart_port, Duration::from_secs(3))?;
        let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
        if send_baud_cmd {
            if let Some(pkt) = uart_tcp_baud_preamble(baud) {
                let _ = (&stream).write_all(&pkt);
            }
        }
        let peer = format!("{}:{}", self.spec.host, self.spec.uart_port);
        self.uart = Some(UartLink::Tcp(stream));
        Ok(Some(format!("UART tcp {peer} @{baud} open")))
    }

    fn uart_close(&mut self) -> Result<Option<String>, InstrumentError> {
        self.uart = None;
        Ok(Some("UART closed".into()))
    }

    fn uart_write(&mut self, data: &str, hex: bool) -> Result<Option<String>, InstrumentError> {
        if self.uart.is_none() && !self.demo {
            self.uart_open(
                String::new(),
                0,
                self.uart_baud,
                self.spec.uart_com.clone(),
                true,
            )?;
        }
        let bytes = if hex {
            parse_hex_bytes(data)?
        } else {
            data.as_bytes().to_vec()
        };
        if bytes.is_empty() {
            return Err(InstrumentError::Io("UART write is empty".into()));
        }
        if self.demo {
            return Ok(Some(format!(
                "UART TX {} B  {}",
                bytes.len(),
                hex_compact(&bytes)
            )));
        }
        match self.uart.as_mut() {
            Some(UartLink::Tcp(stream)) => stream
                .write_all(&bytes)
                .map_err(|e| InstrumentError::Io(format!("UART write: {e}")))?,
            Some(UartLink::Serial(port)) => port
                .write_all(&bytes)
                .map_err(|e| InstrumentError::Io(format!("UART write: {e}")))?,
            Some(UartLink::WinUsb(port)) => port
                .write_all(&bytes)
                .map_err(|e| InstrumentError::Io(format!("UART write: {e}")))?,
            None => return Err(InstrumentError::Io("UART is not open".into())),
        }
        Ok(Some(format!("UART TX {} B", bytes.len())))
    }

    fn uart_read(&mut self) -> Result<Option<String>, InstrumentError> {
        if self.demo || self.uart.is_none() {
            return Ok(None);
        }
        let mut buf = [0u8; 4096];
        let n = match self.uart.as_mut() {
            Some(UartLink::Tcp(stream)) => match stream.read(&mut buf) {
                Ok(n) => n,
                Err(e)
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(InstrumentError::Io(format!("UART read: {e}"))),
            },
            Some(UartLink::Serial(port)) => match port.read(&mut buf) {
                Ok(n) => n,
                Err(e)
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(InstrumentError::Io(format!("UART read: {e}"))),
            },
            Some(UartLink::WinUsb(port)) => match port.read(&mut buf) {
                Ok(n) => n,
                Err(e)
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::Interrupted =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(InstrumentError::Io(format!("UART read: {e}"))),
            },
            None => return Ok(None),
        };
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(format_uart_chunk(&buf[..n])))
    }
}

pub fn parse_dongle_resource(address: &str) -> Option<DongleSpec> {
    let address = address.trim();
    if address.is_empty() {
        return None;
    }
    if let Some(usb) = parse_usb_resource(address) {
        if usb.iface != UsbIface::WiparseDongle && usb.iface != UsbIface::CmsisDap {
            return None;
        }
        let mut spec = DongleSpec {
            serial: usb.serial,
            ..DongleSpec::default()
        };
        apply_query(&mut spec, query_of(address));
        if spec.serial.is_empty() {
            if let Some(rest) = address.strip_prefix("dongle://") {
                let path = rest.split('?').next().unwrap_or("").trim();
                if looks_like_host(path) {
                    spec.host = path.to_owned();
                }
            }
        }
        return Some(spec);
    }
    let rest = address.strip_prefix("dongle://")?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut spec = DongleSpec::default();
    let path = path.trim();
    if let Some((head, serial)) = path.split_once("/serial=") {
        spec.serial = serial.to_owned();
        if looks_like_host(head) {
            spec.host = head.to_owned();
        }
    } else if path.eq_ignore_ascii_case("wiparse")
        || path.eq_ignore_ascii_case("wiparse-dongle")
        || path.eq_ignore_ascii_case("dongle")
        || path.is_empty()
    {
    } else if looks_like_host(path) {
        spec.host = path.to_owned();
    }
    apply_query(&mut spec, query);
    Some(spec)
}

/// Accept a bare host (`192.168.137.123` / `dap.local`) as a TCP-only dongle address.
pub fn parse_dongle_or_host(address: &str) -> Option<DongleSpec> {
    if let Some(spec) = parse_dongle_resource(address) {
        return Some(spec);
    }
    let address = address.trim();
    let (path, query) = address.split_once('?').unwrap_or((address, ""));
    if path.contains("://") {
        return None;
    }
    if looks_like_host(path) {
        let mut spec = DongleSpec {
            host: path.to_owned(),
            ..DongleSpec::default()
        };
        apply_query(&mut spec, query);
        return Some(spec);
    }
    None
}

fn query_of(address: &str) -> &str {
    address.split_once('?').map(|(_, q)| q).unwrap_or("")
}

fn apply_query(spec: &mut DongleSpec, query: &str) {
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let v = urlencoding_lite(v);
        match k.trim().to_ascii_lowercase().as_str() {
            "host" => {
                if !v.is_empty() {
                    spec.host = v;
                }
            }
            "serial" => spec.serial = v,
            "uart" | "uart_port" => {
                if let Ok(p) = v.parse::<u16>() {
                    if p > 0 {
                        spec.uart_port = p;
                    }
                }
            }
            "i2c" | "i2c_port" => {
                if let Ok(p) = v.parse::<u16>() {
                    if p > 0 {
                        spec.i2c_port = p;
                    }
                }
            }
            "com" | "port" => spec.uart_com = v,
            "baud" => {}
            _ => {}
        }
    }
}

fn urlencoding_lite(v: &str) -> String {
    v.replace("%3A", ":")
        .replace("%3a", ":")
        .replace("%20", " ")
}

fn looks_like_host(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("wiparse") {
        return false;
    }
    s.contains('.') || s.chars().all(|c| c.is_ascii_digit() || c == '.') || s.contains(':')
}

fn probe_resource(serial: &str) -> String {
    if serial.is_empty() {
        "probe://cmsis-dap".into()
    } else {
        format!("probe://cmsis-dap/serial={serial}")
    }
}

fn dongle_identity(spec: &DongleSpec, firmware: &str) -> Identity {
    let address = if spec.serial.is_empty() {
        format!("dongle://wiparse?host={}", spec.host)
    } else {
        format!("dongle://wiparse/serial={}", spec.serial)
    };
    Identity {
        manufacturer: "WIPARSE".into(),
        model: "WiParse-dongle".into(),
        serial: if spec.serial.is_empty() {
            spec.host.clone()
        } else {
            spec.serial.clone()
        },
        firmware: firmware.into(),
        raw: address,
    }
}

fn host_candidates(host: &str) -> Vec<String> {
    let raw = host.trim();
    let mut out = Vec::new();
    let mut add = |h: &str| {
        let h = h.trim();
        if !h.is_empty() && !out.iter().any(|x: &String| x.eq_ignore_ascii_case(h)) {
            out.push(h.to_owned());
        }
    };
    if raw.is_empty() {
        add(DEFAULT_MDNS);
        add(DEFAULT_IPV4);
    } else {
        add(raw);
        if raw.eq_ignore_ascii_case(DEFAULT_MDNS) {
            add(DEFAULT_IPV4);
        }
    }
    out
}

/// First `_uart-bridge._tcp` row already in the browse cache. Does not connect
/// to `192.168.137.123` or `dap.local`.
pub fn discover_wifi_uart(_timeout: Duration) -> Option<(String, u16)> {
    crate::mdns_browse::uart_bridge_rows()
        .into_iter()
        .next()
        .map(|row| (row.ipv4.to_string(), row.port))
}

fn connect_tcp(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, InstrumentError> {
    let mut last = String::from("no address");
    for h in host_candidates(host) {
        let addrs = match (h.as_str(), port).to_socket_addrs() {
            Ok(iter) => iter.collect::<Vec<_>>(),
            Err(e) => {
                last = format!("{h}:{port} {e}");
                continue;
            }
        };
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, timeout) {
                Ok(stream) => {
                    let _ = stream.set_nodelay(true);
                    return Ok(stream);
                }
                Err(e) => last = format!("{addr} {e}"),
            }
        }
    }
    Err(InstrumentError::Io(format!(
        "TCP {host}:{port} failed ({last})"
    )))
}

fn read_line(reader: &mut impl Read, timeout: Duration) -> Result<String, InstrumentError> {
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if std::time::Instant::now() > deadline && buf.is_empty() {
            return Err(InstrumentError::Io("I2C reply timeout".into()));
        }
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                if byte[0] != b'\r' {
                    buf.push(byte[0]);
                }
                if buf.len() >= 1024 {
                    break;
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                if buf.is_empty() {
                    return Err(InstrumentError::Io("I2C reply timeout".into()));
                }
                break;
            }
            Err(e) => return Err(InstrumentError::Io(format!("I2C read: {e}"))),
        }
    }
    Ok(String::from_utf8_lossy(&buf).trim().to_owned())
}

fn i2c_addr_token(addr: &str) -> Result<String, InstrumentError> {
    let s = addr.trim();
    if s.is_empty() {
        return Err(InstrumentError::Io("I2C address required".into()));
    }
    let n =
        if s.starts_with("0x") || s.starts_with("0X") || s.chars().all(|c| c.is_ascii_hexdigit()) {
            u32::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16)
                .map_err(|_| InstrumentError::Io(format!("invalid I2C address: {addr}")))?
        } else {
            s.parse::<u32>()
                .map_err(|_| InstrumentError::Io(format!("invalid I2C address: {addr}")))?
        };
    if n > 0x7F {
        return Err(InstrumentError::Io(format!(
            "I2C address out of 7-bit range: {addr}"
        )));
    }
    Ok(format!("{n:02X}"))
}

fn hex_tokens(bytes: &[u8]) -> Result<String, InstrumentError> {
    if bytes.len() > I2C_MAX_FRAME {
        return Err(InstrumentError::Io(format!(
            "I2C payload exceeds {I2C_MAX_FRAME} bytes"
        )));
    }
    Ok(bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" "))
}

fn hex_compact(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

pub fn uart_tcp_baud_preamble(baud: u32) -> Option<Vec<u8>> {
    if !(UART_BAUD_MIN..=UART_BAUD_MAX).contains(&baud) {
        return None;
    }
    let s = baud.to_string();
    if (2..=7).contains(&s.len()) {
        Some(s.into_bytes())
    } else {
        None
    }
}

fn format_uart_chunk(bytes: &[u8]) -> String {
    if bytes
        .iter()
        .all(|b| matches!(b, 0x09 | 0x0A | 0x0D) || (0x20..=0x7E).contains(b))
    {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        hex_compact(bytes)
    }
}

fn demo_i2c_reply(line: &str) -> String {
    let cmd = line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    match cmd.as_str() {
        "HELP" | "?" => "OK HELP SCAN PING SPEED TIMEOUT RECOVER WRITE READ WRITEREAD".into(),
        "SCAN" => "OK SCAN 08 50 77".into(),
        "PING" => "OK PING".into(),
        "RECOVER" | "BUSRESET" => "OK RECOVER".into(),
        "SPEED" => {
            let hz = line.split_whitespace().nth(1).unwrap_or("400000");
            format!("OK SPEED {hz}")
        }
        "TIMEOUT" => {
            let ms = line.split_whitespace().nth(1).unwrap_or("50");
            format!("OK TIMEOUT {ms}")
        }
        "WRITE" => "OK WRITE".into(),
        "READ" => "OK READ AA 55".into(),
        "WRITEREAD" => "OK WRITEREAD AA 55".into(),
        _ => format!("OK {cmd}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applying_i2c_endpoint_preserves_uart_and_identical_settings_keep_both_links() {
        fn socket_pair() -> (TcpStream, TcpStream) {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (server, _) = listener.accept().unwrap();
            (client, server)
        }
        let mut session = DongleSession::demo("dongle://wiparse");
        let (uart, mut uart_peer) = socket_pair();
        let (i2c, _i2c_peer) = socket_pair();
        session.uart = Some(UartLink::Tcp(uart));
        session.i2c = Some(I2cLink::Tcp(i2c));
        let host = session.spec.host.clone();
        let uart_port = session.spec.uart_port;
        let i2c_port = session.spec.i2c_port;
        session.set_link(host.clone(), uart_port, i2c_port).unwrap();
        assert!(session.uart.is_some());
        assert!(session.i2c.is_some());
        session
            .set_link(host.clone(), uart_port, i2c_port + 1)
            .unwrap();
        assert!(session.i2c.is_none());
        let Some(UartLink::Tcp(link)) = &mut session.uart else {
            panic!("I2C change closed UART");
        };
        link.write_all(b"alive").unwrap();
        uart_peer
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut bytes = [0; 5];
        uart_peer.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"alive");
        let (i2c, _peer) = socket_pair();
        session.i2c = Some(I2cLink::Tcp(i2c));
        session.set_link(host, uart_port + 1, i2c_port + 1).unwrap();
        assert!(session.uart.is_none());
        assert!(session.i2c.is_some());
        session
            .set_link("192.0.2.99".into(), uart_port + 1, i2c_port + 1)
            .unwrap();
        assert!(session.i2c.is_none());
    }

    #[test]
    fn parses_dongle_urls() {
        let a = parse_dongle_resource("dongle://wiparse/serial=ABC").unwrap();
        assert_eq!(a.serial, "ABC");
        assert_eq!(a.host, DEFAULT_IPV4);
        let b = parse_dongle_resource("dongle://dap.local?uart=1234&i2c=1236").unwrap();
        assert_eq!(b.host, "dap.local");
        let c = parse_dongle_resource("dongle://wiparse/serial=X?host=10.0.0.8&com=COM7").unwrap();
        assert_eq!(c.host, "10.0.0.8");
        assert_eq!(c.uart_com, "COM7");
    }

    #[test]
    fn demo_i2c_and_uart() {
        let mut d = DongleSession::demo("dongle://wiparse/serial=DEMO");
        let scan = d.execute(ControlCommand::DongleI2cScan).unwrap().unwrap();
        assert!(scan.contains("OK SCAN"));
        let info = d.execute(ControlCommand::DongleInfo).unwrap().unwrap();
        assert!(info.contains(IF_ID));
        assert!(info.contains("I2C"));
        d.execute(ControlCommand::DongleUartOpen {
            host: String::new(),
            port: 0,
            baud: 115200,
            com: String::new(),
            send_baud_cmd: true,
        })
        .unwrap();
        let tx = d
            .execute(ControlCommand::DongleUartWrite {
                data: "AA55".into(),
                hex: true,
            })
            .unwrap()
            .unwrap();
        assert!(tx.contains("TX"));
        assert!(d
            .execute(ControlCommand::ProbeStatus)
            .unwrap()
            .unwrap()
            .to_ascii_lowercase()
            .contains("halt"));
    }
}
