//! Windows UART diagnostics for a station-owned, exclusively opened COM port.
//! The absence of OS-reported errors does not prove that a USB adapter or the
//! upstream transmitter did not silently drop data.
use std::{io::{self, Read}, os::windows::io::AsRawHandle, time::Duration};
use windows_sys::Win32::Devices::Communication::{ClearCommError, COMSTAT};

pub(crate) struct OwnedUart {
    port: serialport::COMPort,
    resource: String,
    latched_errors: u32,
}

#[derive(Debug)]
pub(crate) struct UartChunk {
    pub bytes: Vec<u8>,
    pub windows_error_flags: u32,
    /// Windows flags alone do not verify transmitter/adapter continuity.
    pub continuity_verified: bool,
}

impl OwnedUart {
    /// serialport's Windows implementation opens with share mode zero. Do not
    /// clone the handle or allow another reader to clear its error flags.
    pub(crate) fn open(resource: &str, baud: u32, timeout: Duration) -> Result<Self, String> {
        if resource.trim().is_empty() || !(1200..=2_000_000).contains(&baud)
            || timeout.is_zero() || timeout > Duration::from_secs(1) {
            return Err("STATION.UART_CONFIG_INVALID".into());
        }
        let port = serialport::new(resource, baud).timeout(timeout).open_native()
            .map_err(|e| format!("STATION.UART_OPEN: {e}"))?;
        let mut owned = Self { port, resource: resource.to_owned(), latched_errors: 0 };
        owned.sample_errors()?;
        if owned.latched_errors != 0 { return Err("STATION.UART_INITIAL_ERRORS".into()); }
        Ok(owned)
    }

    pub(crate) fn resource(&self) -> &str { &self.resource }

    fn sample_errors(&mut self) -> Result<(), String> {
        let mut flags = 0u32;
        let mut status: COMSTAT = unsafe { std::mem::zeroed() };
        // ClearCommError reads *and clears* the driver's latched error flags.
        // Exclusive ownership and accumulating every returned flag are required.
        let ok = unsafe { ClearCommError(self.port.as_raw_handle() as isize, &mut flags, &mut status) };
        if ok == 0 { return Err(format!("STATION.UART_STATUS: {}", io::Error::last_os_error())); }
        self.latched_errors |= flags;
        Ok(())
    }

    pub(crate) fn read_checked(&mut self) -> Result<UartChunk, String> {
        self.sample_errors()?;
        if self.latched_errors != 0 { return Err(format!("STATION.UART_LINE_ERRORS: 0x{:x}", self.latched_errors)); }
        let mut data = vec![0; 8192];
        let count = match self.port.read(&mut data) {
            Ok(0) => return Err("STATION.UART_DISCONNECTED".into()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::TimedOut => 0,
            Err(e) => return Err(format!("STATION.UART_READ: {e}")),
        };
        self.sample_errors()?;
        if self.latched_errors != 0 { return Err(format!("STATION.UART_LINE_ERRORS: 0x{:x}", self.latched_errors)); }
        data.truncate(count);
        Ok(UartChunk { bytes: data, windows_error_flags: self.latched_errors, continuity_verified: false })
    }
}
