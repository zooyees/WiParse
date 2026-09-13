//! FTDI FT4222H USB-SPI/I2C/GPIO bridge (optional LibFT4222.dll).

use crate::instrument::{ControlCommand, Identity, InstrumentError, InstrumentKind};
use crate::usb::{parse_usb_resource, UsbIface, UsbResource};
use libloading::{Library, Symbol};
use std::ffi::{c_void, CString};
use std::sync::Mutex;

const FT_OK: u32 = 0;
const FT_OPEN_BY_SERIAL_NUMBER: u32 = 1;

#[repr(C)]
struct Ft4222Version {
    chip: u32,
    dll: u32,
}

pub struct Ft4222Session {
    pub resource: String,
    pub identity: Identity,
    pub kind: InstrumentKind,
    inner: BridgeInner,
}

enum BridgeInner {
    Demo {
        gpio_dir: [bool; 4],
        gpio_val: [bool; 4],
    },
    Live(LiveBridge),
}

struct LiveBridge {
    _d2xx: Library,
    _ft4222: Library,
    handle: *mut c_void,
    ft_close: unsafe extern "system" fn(*mut c_void) -> u32,
    uninit: unsafe extern "system" fn(*mut c_void) -> u32,
    spi_init: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32, u8) -> u32,
    spi_xfer: unsafe extern "system" fn(*mut c_void, *mut u8, *mut u8, u16, *mut u16, i32) -> u32,
    i2c_init: unsafe extern "system" fn(*mut c_void, u32) -> u32,
    i2c_write: unsafe extern "system" fn(*mut c_void, u16, *const u8, u16, *mut u16) -> u32,
    i2c_read: unsafe extern "system" fn(*mut c_void, u16, *mut u8, u16, *mut u16) -> u32,
    gpio_init: unsafe extern "system" fn(*mut c_void, *const u32) -> u32,
    gpio_read: unsafe extern "system" fn(*mut c_void, u32, *mut i32) -> u32,
    gpio_write: unsafe extern "system" fn(*mut c_void, u32, i32) -> u32,
    set_suspend_out: unsafe extern "system" fn(*mut c_void, i32) -> u32,
    set_wakeup: unsafe extern "system" fn(*mut c_void, i32) -> u32,
    get_version: unsafe extern "system" fn(*mut c_void, *mut Ft4222Version) -> u32,
}

unsafe impl Send for LiveBridge {}

impl Drop for LiveBridge {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.uninit)(self.handle);
            let _ = (self.ft_close)(self.handle);
        }
    }
}

static FT_OPEN_LOCK: Mutex<()> = Mutex::new(());

impl Ft4222Session {
    pub fn demo(resource: impl Into<String>) -> Self {
        let resource = resource.into();
        let spec = parse_usb_resource(&resource).unwrap_or(UsbResource {
            iface: UsbIface::Ft4222,
            serial: "DEMO".into(),
        });
        let mut identity = spec.identity();
        identity.serial = if identity.serial.is_empty() {
            "DEMO".into()
        } else {
            identity.serial
        };
        identity.firmware = "demo".into();
        Self {
            resource,
            identity,
            kind: InstrumentKind::UsbBridge,
            inner: BridgeInner::Demo {
                gpio_dir: [true, true, true, true],
                gpio_val: [false; 4],
            },
        }
    }

    pub fn open(resource: &str) -> Result<Self, InstrumentError> {
        let spec = parse_usb_resource(resource).ok_or_else(|| {
            InstrumentError::Io(format!("not a bridge resource: {resource}"))
        })?;
        if spec.iface != UsbIface::Ft4222 {
            return Err(InstrumentError::Unsupported(
                "only FT4222 USB bridges are supported here".into(),
            ));
        }
        let live = LiveBridge::open(if spec.serial.is_empty() {
            None
        } else {
            Some(spec.serial.as_str())
        })?;
        Ok(Self {
            resource: resource.to_owned(),
            identity: spec.identity(),
            kind: InstrumentKind::UsbBridge,
            inner: BridgeInner::Live(live),
        })
    }

    pub fn execute(
        &mut self,
        command: ControlCommand,
    ) -> Result<Option<String>, InstrumentError> {
        match command {
            ControlCommand::BridgeSpi {
                mode,
                clock_hz,
                cs,
                write_hex,
                read_len,
            } => self.spi(mode, clock_hz, cs, &write_hex, read_len),
            ControlCommand::BridgeI2c {
                addr,
                write_hex,
                read_len,
                clock_hz,
            } => self.i2c(&addr, &write_hex, read_len, clock_hz),
            ControlCommand::BridgeGpio { pin, dir, value } => self.gpio(pin, dir, value),
            ControlCommand::BridgeInfo => self.info(),
            other => Err(InstrumentError::Unsupported(format!(
                "FT4222 cannot run {other:?}"
            ))),
        }
    }

    fn spi(
        &mut self,
        mode: u8,
        clock_hz: u32,
        _cs: u8,
        write_hex: &str,
        read_len: u32,
    ) -> Result<Option<String>, InstrumentError> {
        let mut write = if write_hex.trim().is_empty() {
            vec![0u8; read_len.max(1) as usize]
        } else {
            crate::probe::parse_hex_bytes(write_hex)?
        };
        if read_len as usize > write.len() {
            write.resize(read_len as usize, 0);
        }
        match &mut self.inner {
            BridgeInner::Demo { .. } => Ok(Some(format!(
                "SPI mode={mode} clk={clock_hz} MOSI {}  MISO {}",
                hex_compact(&write),
                hex_compact(&write)
            ))),
            BridgeInner::Live(live) => {
                live.spi_init(mode, clock_hz)?;
                let read = live.spi_xfer(&write)?;
                Ok(Some(format!(
                    "SPI MOSI {}  MISO {}",
                    hex_compact(&write),
                    hex_compact(&read)
                )))
            }
        }
    }

    fn i2c(
        &mut self,
        addr: &str,
        write_hex: &str,
        read_len: u32,
        clock_hz: u32,
    ) -> Result<Option<String>, InstrumentError> {
        let address = parse_i2c_addr(addr)?;
        let write = if write_hex.trim().is_empty() {
            Vec::new()
        } else {
            crate::probe::parse_hex_bytes(write_hex)?
        };
        match &mut self.inner {
            BridgeInner::Demo { .. } => {
                let dummy = vec![0u8; read_len as usize];
                Ok(Some(format!(
                    "I2C 0x{address:02X} @{clock_hz}Hz write {} read {}",
                    hex_compact(&write),
                    hex_compact(&dummy)
                )))
            }
            BridgeInner::Live(live) => {
                live.i2c_init(clock_hz)?;
                if !write.is_empty() {
                    live.i2c_write(address, &write)?;
                }
                let read = if read_len > 0 {
                    live.i2c_read(address, read_len as usize)?
                } else {
                    Vec::new()
                };
                Ok(Some(format!(
                    "I2C 0x{address:02X} @{clock_hz}Hz write {} read {}",
                    hex_compact(&write),
                    hex_compact(&read)
                )))
            }
        }
    }

    fn info(&self) -> Result<Option<String>, InstrumentError> {
        match &self.inner {
            BridgeInner::Demo { gpio_dir, gpio_val } => Ok(Some(format!(
                "FT4222 DEMO serial={} gpio_dir={gpio_dir:?} gpio_val={gpio_val:?}",
                self.identity.serial
            ))),
            BridgeInner::Live(live) => live.info(),
        }
    }

    fn gpio(
        &mut self,
        pin: u8,
        dir: Option<bool>,
        value: Option<bool>,
    ) -> Result<Option<String>, InstrumentError> {
        match &mut self.inner {
            BridgeInner::Demo {
                gpio_dir,
                gpio_val,
            } => {
                if pin > 3 {
                    return Ok(Some(format!(
                        "GPIO {}",
                        (0..4)
                            .map(|i| format!(
                                "P{i}:{}={}",
                                if gpio_dir[i] { "out" } else { "in" },
                                u8::from(gpio_val[i])
                            ))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )));
                }
                let pin = pin as usize;
                if let Some(d) = dir {
                    gpio_dir[pin] = d;
                }
                if let Some(v) = value {
                    gpio_val[pin] = v;
                }
                Ok(Some(format!(
                    "GPIO{pin} dir={} val={}",
                    if gpio_dir[pin] { "out" } else { "in" },
                    u8::from(gpio_val[pin])
                )))
            }
            BridgeInner::Live(live) => live.gpio(pin, dir, value),
        }
    }
}

impl LiveBridge {
    fn open(serial: Option<&str>) -> Result<Self, InstrumentError> {
        let _guard = FT_OPEN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d2xx = crate::native_lib::load_named(crate::native_lib::ftd2xx_dll_names())
            .map_err(|_| missing_dll())?;
        let ft4222 = crate::native_lib::load_named(crate::native_lib::ftdi_dll_names())
            .map_err(|_| missing_dll())?;
        unsafe {
            let ft_open_ex: Symbol<
                unsafe extern "system" fn(*const u8, u32, *mut *mut c_void) -> u32,
            > = d2xx.get(b"FT_OpenEx\0").map_err(|e| InstrumentError::Io(e.to_string()))?;
            let ft_open: Symbol<unsafe extern "system" fn(i32, *mut *mut c_void) -> u32> =
                d2xx.get(b"FT_Open\0").map_err(|e| InstrumentError::Io(e.to_string()))?;
            let ft_close: Symbol<unsafe extern "system" fn(*mut c_void) -> u32> =
                d2xx.get(b"FT_Close\0").map_err(|e| InstrumentError::Io(e.to_string()))?;
            let uninit: Symbol<unsafe extern "system" fn(*mut c_void) -> u32> = ft4222
                .get(b"FT4222_UnInitialize\0")
                .map_err(|e| InstrumentError::Io(e.to_string()))?;
            let spi_init: Symbol<
                unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32, u8) -> u32,
            > = ft4222
                .get(b"FT4222_SPIMaster_Init\0")
                .map_err(|e| InstrumentError::Io(e.to_string()))?;
            let spi_xfer: Symbol<
                unsafe extern "system" fn(*mut c_void, *mut u8, *mut u8, u16, *mut u16, i32) -> u32,
            > = ft4222
                .get(b"FT4222_SPIMaster_SingleReadWrite\0")
                .map_err(|e| InstrumentError::Io(e.to_string()))?;
            let i2c_init: Symbol<unsafe extern "system" fn(*mut c_void, u32) -> u32> = ft4222
                .get(b"FT4222_I2CMaster_Init\0")
                .map_err(|e| InstrumentError::Io(e.to_string()))?;
            let i2c_write: Symbol<
                unsafe extern "system" fn(*mut c_void, u16, *const u8, u16, *mut u16) -> u32,
            > = ft4222
                .get(b"FT4222_I2CMaster_Write\0")
                .map_err(|e| InstrumentError::Io(e.to_string()))?;
            let i2c_read: Symbol<
                unsafe extern "system" fn(*mut c_void, u16, *mut u8, u16, *mut u16) -> u32,
            > = ft4222
                .get(b"FT4222_I2CMaster_Read\0")
                .map_err(|e| InstrumentError::Io(e.to_string()))?;
            let gpio_init: Symbol<unsafe extern "system" fn(*mut c_void, *const u32) -> u32> =
                ft4222
                    .get(b"FT4222_GPIO_Init\0")
                    .map_err(|e| InstrumentError::Io(e.to_string()))?;
            let gpio_read: Symbol<unsafe extern "system" fn(*mut c_void, u32, *mut i32) -> u32> =
                ft4222
                    .get(b"FT4222_GPIO_Read\0")
                    .map_err(|e| InstrumentError::Io(e.to_string()))?;
            let gpio_write: Symbol<unsafe extern "system" fn(*mut c_void, u32, i32) -> u32> =
                ft4222
                    .get(b"FT4222_GPIO_Write\0")
                    .map_err(|e| InstrumentError::Io(e.to_string()))?;
            let set_suspend_out: Symbol<unsafe extern "system" fn(*mut c_void, i32) -> u32> =
                ft4222
                    .get(b"FT4222_SetSuspendOut\0")
                    .map_err(|e| InstrumentError::Io(e.to_string()))?;
            let set_wakeup: Symbol<unsafe extern "system" fn(*mut c_void, i32) -> u32> = ft4222
                .get(b"FT4222_SetWakeUpInterrupt\0")
                .map_err(|e| InstrumentError::Io(e.to_string()))?;

            let get_version: Symbol<
                unsafe extern "system" fn(*mut c_void, *mut Ft4222Version) -> u32,
            > = ft4222
                .get(b"FT4222_GetVersion\0")
                .map_err(|e| InstrumentError::Io(e.to_string()))?;

            let mut handle: *mut c_void = std::ptr::null_mut();
            let status = if let Some(serial) = serial {
                let c = CString::new(serial).map_err(|e| InstrumentError::Io(e.to_string()))?;
                ft_open_ex(c.as_ptr() as *const u8, FT_OPEN_BY_SERIAL_NUMBER, &mut handle)
            } else {
                ft_open(0, &mut handle)
            };
            if status != FT_OK || handle.is_null() {
                return Err(InstrumentError::Io(format!(
                    "FT_Open failed status={status}. Is LibFT4222.dll / ftd2xx.dll installed?"
                )));
            }

            Ok(Self {
                ft_close: *ft_close,
                uninit: *uninit,
                spi_init: *spi_init,
                spi_xfer: *spi_xfer,
                i2c_init: *i2c_init,
                i2c_write: *i2c_write,
                i2c_read: *i2c_read,
                gpio_init: *gpio_init,
                gpio_read: *gpio_read,
                gpio_write: *gpio_write,
                set_suspend_out: *set_suspend_out,
                set_wakeup: *set_wakeup,
                get_version: *get_version,
                handle,
                _d2xx: d2xx,
                _ft4222: ft4222,
            })
        }
    }

    fn spi_init(&self, mode: u8, clock_hz: u32) -> Result<(), InstrumentError> {
        let (cpol, cpha) = match mode & 0x03 {
            0 => (0, 0),
            1 => (0, 1),
            2 => (1, 0),
            _ => (1, 1),
        };
        let clk = spi_clock_div(clock_hz);
        let st = unsafe { (self.spi_init)(self.handle, 1, clk, cpol, cpha, 0x01) };
        ft_ok(st, "FT4222_SPIMaster_Init")
    }

    fn spi_xfer(&self, write: &[u8]) -> Result<Vec<u8>, InstrumentError> {
        let mut read = vec![0u8; write.len()];
        let mut write_buf = write.to_vec();
        let mut xfer: u16 = 0;
        let st = unsafe {
            (self.spi_xfer)(
                self.handle,
                read.as_mut_ptr(),
                write_buf.as_mut_ptr(),
                write.len() as u16,
                &mut xfer,
                1,
            )
        };
        ft_ok(st, "FT4222_SPIMaster_SingleReadWrite")?;
        read.truncate(xfer as usize);
        Ok(read)
    }

    fn i2c_init(&self, clock_hz: u32) -> Result<(), InstrumentError> {
        let kbps = (clock_hz / 1000).clamp(60, 3400);
        ft_ok(unsafe { (self.i2c_init)(self.handle, kbps) }, "FT4222_I2CMaster_Init")
    }

    fn i2c_write(&self, addr: u16, data: &[u8]) -> Result<(), InstrumentError> {
        let mut n: u16 = 0;
        ft_ok(
            unsafe { (self.i2c_write)(self.handle, addr, data.as_ptr(), data.len() as u16, &mut n) },
            "FT4222_I2CMaster_Write",
        )
    }

    fn i2c_read(&self, addr: u16, len: usize) -> Result<Vec<u8>, InstrumentError> {
        let mut buf = vec![0u8; len];
        let mut n: u16 = 0;
        ft_ok(
            unsafe { (self.i2c_read)(self.handle, addr, buf.as_mut_ptr(), len as u16, &mut n) },
            "FT4222_I2CMaster_Read",
        )?;
        buf.truncate(n as usize);
        Ok(buf)
    }

    fn gpio(
        &self,
        pin: u8,
        dir: Option<bool>,
        value: Option<bool>,
    ) -> Result<Option<String>, InstrumentError> {
        unsafe {
            let _ = (self.set_suspend_out)(self.handle, 0);
            let _ = (self.set_wakeup)(self.handle, 0);
        }
        let mut dirs = [1u32; 4];
        if pin <= 3 {
            if let Some(false) = dir {
                dirs[pin as usize] = 0;
            }
        }
        ft_ok(unsafe { (self.gpio_init)(self.handle, dirs.as_ptr()) }, "FT4222_GPIO_Init")?;
        if pin <= 3 {
            if let Some(v) = value {
                ft_ok(
                    unsafe { (self.gpio_write)(self.handle, pin as u32, i32::from(v)) },
                    "FT4222_GPIO_Write",
                )?;
            }
            let mut read = 0i32;
            ft_ok(
                unsafe { (self.gpio_read)(self.handle, pin as u32, &mut read) },
                "FT4222_GPIO_Read",
            )?;
            return Ok(Some(format!("GPIO{pin}={read}")));
        }
        let mut parts = Vec::new();
        for p in 0..4u32 {
            let mut read = 0i32;
            ft_ok(
                unsafe { (self.gpio_read)(self.handle, p, &mut read) },
                "FT4222_GPIO_Read",
            )?;
            parts.push(format!("P{p}={read}"));
        }
        Ok(Some(format!("GPIO {}", parts.join(" "))))
    }

    fn info(&self) -> Result<Option<String>, InstrumentError> {
        let mut ver = Ft4222Version { chip: 0, dll: 0 };
        ft_ok(
            unsafe { (self.get_version)(self.handle, &mut ver) },
            "FT4222_GetVersion",
        )?;
        Ok(Some(format!(
            "FT4222 chip=0x{:08X} dll=0x{:08X}  {}",
            ver.chip,
            ver.dll,
            crate::native_lib::ftdi_runtime_report().replace('\n', " | ")
        )))
    }
}

fn missing_dll() -> InstrumentError {
    InstrumentError::Io(crate::native_lib::ftdi_runtime_report())
}

fn ft_ok(status: u32, what: &str) -> Result<(), InstrumentError> {
    if status == FT_OK {
        Ok(())
    } else {
        Err(InstrumentError::Io(format!("{what} failed status={status}")))
    }
}

fn spi_clock_div(hz: u32) -> u32 {
    // FT4222_SPIClock: 1=CLK_DIV_2 … 9=CLK_DIV_512 (sys ~80 MHz).
    match hz {
        0..=200_000 => 9,
        201_000..=400_000 => 8,
        401_000..=800_000 => 7,
        801_000..=1_500_000 => 6,
        1_500_001..=3_000_000 => 5,
        3_000_001..=6_000_000 => 4,
        6_000_001..=12_000_000 => 3,
        12_000_001..=24_000_000 => 2,
        _ => 1,
    }
}

fn parse_i2c_addr(text: &str) -> Result<u16, InstrumentError> {
    let t = text.trim().trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(t, 16)
        .or_else(|_| t.parse::<u16>())
        .map_err(|_| InstrumentError::Io(format!("invalid I2C address: {text}")))
}

fn hex_compact(data: &[u8]) -> String {
    if data.is_empty() {
        return "—".into();
    }
    data.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_bridge_spi() {
        let mut bridge = Ft4222Session::demo("bridge://ft4222/serial=DEMO");
        let out = bridge
            .execute(ControlCommand::BridgeSpi {
                mode: 0,
                clock_hz: 1_000_000,
                cs: 0,
                write_hex: "9F".into(),
                read_len: 4,
            })
            .unwrap()
            .unwrap();
        assert!(out.contains("SPI"));
    }
}
