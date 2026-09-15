//! WIPARSE UART over WinUSB (ICD CH-UART MI_01).
//!
//! Device Manager lists this under "Universal Serial Bus devices", not
//! "Ports (COM & LPT)". The serial tool treats it like a COM session:
//! Start/baud/log, bulk IN/OUT plus CDC SET_LINE_CODING. Windows opens via
//! ICD 1.1 DeviceInterfaceGUID `{7B3E9C41-…0101}` (same as Hub
//! `winusb-host.cs`). `GLOBALROOT` remains a fallback for older firmware.

use crate::serial::{PortInfo, SerialError};
use std::io::{self, Read, Write};

const VID: u16 = 0xC251;
const PID: u16 = 0xF01A;
const UART_MI: u8 = 1;
const UART_OUT_EP: u8 = 0x02;
const UART_IN_EP: u8 = 0x83;
const I2C_MI: u8 = 3;
const I2C_OUT_EP: u8 = 0x03;
const I2C_IN_EP: u8 = 0x84;
pub const WINUSB_UART_PREFIX: &str = "winusb://wiparse/uart";

#[cfg(windows)]
#[path = "serial_winusb_win.rs"]
mod win;

pub fn list_winusb_uart_ports() -> Vec<PortInfo> {
    let mut out = list_from_nusb();
    #[cfg(windows)]
    if out.is_empty() && win::uart_function_present() {
        out.push(uart_port_info(""));
    }
    out
}

fn list_from_nusb() -> Vec<PortInfo> {
    let Ok(iter) = nusb::list_devices() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dev in iter {
        if dev.vendor_id() != VID || dev.product_id() != PID {
            continue;
        }
        let serial = dev.serial_number().unwrap_or_default().to_owned();
        let info = uart_port_info(&serial);
        if out.iter().any(|p: &PortInfo| p.device == info.device) {
            continue;
        }
        out.push(info);
    }
    out
}

fn uart_port_info(serial: &str) -> PortInfo {
    let device = if serial.is_empty() {
        WINUSB_UART_PREFIX.to_string()
    } else {
        format!("{WINUSB_UART_PREFIX}?serial={serial}")
    };
    PortInfo {
        label: "WIPARSE UART · USB".into(),
        description: "WIPARSE UART".into(),
        hwid: format!("USB VID:PID={VID:04X}:{PID:04X} MI_{UART_MI:02}"),
        transport: "usb".into(),
        device,
    }
}

pub fn parse_winusb_uart_device(device: &str) -> Option<String> {
    let raw = device.trim();
    let rest = raw
        .strip_prefix(WINUSB_UART_PREFIX)
        .or_else(|| raw.strip_prefix("WINUSB://wiparse/uart"))?;
    if rest.is_empty() {
        return Some(String::new());
    }
    let q = rest.strip_prefix('?')?;
    for part in q.split('&') {
        if let Some(s) = part.strip_prefix("serial=") {
            return Some(s.to_string());
        }
    }
    Some(String::new())
}

pub fn open_winusb_uart(_serial: &str, baud: u32) -> Result<WinUsbUart, SerialError> {
    #[cfg(windows)]
    {
        match win::NativeUart::open() {
            Ok(native) => {
                native.apply_line_coding(baud);
                return Ok(WinUsbUart {
                    inner: Inner::Native(native),
                });
            }
            Err(setupapi_err) => {
                match open_nusb(_serial, baud) {
                    Ok(uart) => return Ok(uart),
                    Err(_) => {
                        return Err(SerialError::Message(setupapi_err));
                    }
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        open_nusb(_serial, baud)
    }
}

pub fn open_winusb_i2c(_serial: &str) -> Result<WinUsbI2c, SerialError> {
    #[cfg(windows)]
    {
        match win::NativeI2c::open() {
            Ok(native) => {
                return Ok(WinUsbI2c {
                    inner: InnerI2c::Native(native),
                });
            }
            Err(setupapi_err) => match open_nusb_i2c(_serial) {
                Ok(i2c) => return Ok(i2c),
                Err(_) => return Err(SerialError::Message(setupapi_err)),
            },
        }
    }
    #[cfg(not(windows))]
    {
        open_nusb_i2c(_serial)
    }
}

fn open_nusb_i2c(serial: &str) -> Result<WinUsbI2c, SerialError> {
    let mut found = None;
    let iter = nusb::list_devices().map_err(|e| SerialError::Message(e.to_string()))?;
    for dev in iter {
        if dev.vendor_id() != VID || dev.product_id() != PID {
            continue;
        }
        let sn = dev.serial_number().unwrap_or("");
        if serial.is_empty() || sn.eq_ignore_ascii_case(serial) {
            found = Some(dev);
            break;
        }
    }
    let info = found.ok_or_else(|| SerialError::Message("WIPARSE I2C USB device not found".into()))?;
    let mut mi = I2C_MI;
    for i in info.interfaces() {
        let name = i.interface_string().unwrap_or("").to_ascii_uppercase();
        if name.contains("I2C") {
            mi = i.interface_number();
            break;
        }
    }
    let device = info
        .open()
        .map_err(|e| SerialError::Message(format!("open WIPARSE I2C: {e}")))?;
    let iface = device.claim_interface(mi).map_err(|e| {
        SerialError::Message(format!(
            "WIPARSE I2C is WinUSB MI_{mi:02}. Claim failed: {e}."
        ))
    })?;
    Ok(WinUsbI2c {
        inner: InnerI2c::Nusb { iface },
    })
}

fn open_nusb(serial: &str, baud: u32) -> Result<WinUsbUart, SerialError> {
    let mut found = None;
    let iter = nusb::list_devices().map_err(|e| SerialError::Message(e.to_string()))?;
    for dev in iter {
        if dev.vendor_id() != VID || dev.product_id() != PID {
            continue;
        }
        let sn = dev.serial_number().unwrap_or("");
        if serial.is_empty() || sn.eq_ignore_ascii_case(serial) {
            found = Some(dev);
            break;
        }
    }
    let info = found.ok_or_else(|| SerialError::Message("WIPARSE UART USB device not found".into()))?;
    let mut mi = UART_MI;
    for i in info.interfaces() {
        let name = i.interface_string().unwrap_or("").to_ascii_uppercase();
        if name.contains("UART") {
            mi = i.interface_number();
            break;
        }
    }
    let device = info
        .open()
        .map_err(|e| SerialError::Message(format!("open WIPARSE UART: {e}")))?;
    let iface = device.claim_interface(mi).map_err(|e| {
        SerialError::Message(format!(
            "WIPARSE UART is WinUSB MI_{mi:02}, not a COM port. Claim failed: {e}."
        ))
    })?;
    let uart = WinUsbUart {
        inner: Inner::Nusb { iface },
    };
    uart.apply_nusb_line_coding(baud);
    Ok(uart)
}

enum Inner {
    Nusb { iface: nusb::Interface },
    #[cfg(windows)]
    Native(win::NativeUart),
}

pub struct WinUsbUart {
    inner: Inner,
}

impl WinUsbUart {
    fn apply_nusb_line_coding(&self, baud: u32) {
        use nusb::transfer::{Control, ControlType, Recipient};
        let Inner::Nusb { iface } = &self.inner else {
            return;
        };
        let baud = baud.clamp(1_200, 2_000_000);
        let mut coding = [0u8; 7];
        coding[0..4].copy_from_slice(&baud.to_le_bytes());
        coding[4] = 0;
        coding[5] = 0;
        coding[6] = 8;
        let _ = iface.control_out_blocking(
            Control {
                control_type: ControlType::Class,
                recipient: Recipient::Interface,
                request: 0x20,
                value: 0,
                index: UART_MI as u16,
            },
            &coding,
            std::time::Duration::from_millis(200),
        );
        let _ = iface.control_out_blocking(
            Control {
                control_type: ControlType::Class,
                recipient: Recipient::Interface,
                request: 0x22,
                value: 1,
                index: UART_MI as u16,
            },
            &[],
            std::time::Duration::from_millis(200),
        );
    }
}

impl Read for WinUsbUart {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match &mut self.inner {
            Inner::Nusb { iface } => nusb_read(iface, UART_IN_EP, buf, std::time::Duration::from_millis(50)),
            #[cfg(windows)]
            Inner::Native(n) => n.read(buf),
        }
    }
}

impl Write for WinUsbUart {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.inner {
            Inner::Nusb { iface } => nusb_write(iface, UART_OUT_EP, buf),
            #[cfg(windows)]
            Inner::Native(n) => n.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

enum InnerI2c {
    Nusb { iface: nusb::Interface },
    #[cfg(windows)]
    Native(win::NativeI2c),
}

pub struct WinUsbI2c {
    inner: InnerI2c,
}

impl Read for WinUsbI2c {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match &mut self.inner {
            InnerI2c::Nusb { iface } => {
                nusb_read(iface, I2C_IN_EP, buf, std::time::Duration::from_millis(2_500))
            }
            #[cfg(windows)]
            InnerI2c::Native(n) => n.read(buf),
        }
    }
}

impl Write for WinUsbI2c {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.inner {
            InnerI2c::Nusb { iface } => nusb_write(iface, I2C_OUT_EP, buf),
            #[cfg(windows)]
            InnerI2c::Native(n) => n.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn nusb_read(
    iface: &nusb::Interface,
    ep: u8,
    buf: &mut [u8],
    timeout: std::time::Duration,
) -> io::Result<usize> {
    use nusb::transfer::{RequestBuffer, TransferError};

    if buf.is_empty() {
        return Ok(0);
    }
    let n = ((buf.len().min(4096) + 63) / 64 * 64).max(64);
    let fut = iface.bulk_in(ep, RequestBuffer::new(n));
    let Some(completion) = block_on_timeout(fut, timeout) else {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "timeout"));
    };
    match completion.status {
        Ok(()) => {
            let n = completion.data.len().min(buf.len());
            buf[..n].copy_from_slice(&completion.data[..n]);
            Ok(n)
        }
        Err(TransferError::Cancelled) => Err(io::Error::new(io::ErrorKind::TimedOut, "timeout")),
        Err(e) => Err(e.into()),
    }
}

fn nusb_write(iface: &nusb::Interface, ep: u8, buf: &[u8]) -> io::Result<usize> {
    use std::time::Duration;
    if buf.is_empty() {
        return Ok(0);
    }
    let fut = iface.bulk_out(ep, buf.to_vec());
    let Some(completion) = block_on_timeout(fut, Duration::from_secs(2)) else {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "usb write timeout"));
    };
    completion.status.map_err(io::Error::from)?;
    Ok(completion.data.actual_length())
}

fn block_on_timeout<T>(fut: impl std::future::Future<Output = T>, timeout: std::time::Duration) -> Option<T> {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    use std::thread;
    use std::time::Instant;

    struct ThreadWake(thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    let deadline = Instant::now() + timeout;
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return Some(v);
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        thread::park_timeout(deadline.saturating_duration_since(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_winusb_uart_device_ok() {
        assert_eq!(parse_winusb_uart_device("winusb://wiparse/uart").as_deref(), Some(""));
        assert_eq!(
            parse_winusb_uart_device("winusb://wiparse/uart?serial=ABC").as_deref(),
            Some("ABC")
        );
        assert!(parse_winusb_uart_device("COM13").is_none());
        assert!(parse_winusb_uart_device("tcp://192.168.137.123:1234").is_none());
    }

    #[test]
    fn winusb_uart_lists_when_dongle_present() {
        let dongles: Vec<_> = crate::usb::discover_usb_instruments()
            .into_iter()
            .filter(|r| r.kind == Some(crate::instrument::InstrumentKind::WiparseDongle))
            .collect();
        let uart = list_winusb_uart_ports();
        if !dongles.is_empty() {
            assert!(
                !uart.is_empty(),
                "dongle {:?} present but UART list is empty",
                dongles.iter().map(|d| d.address.clone()).collect::<Vec<_>>()
            );
        }
    }
}
