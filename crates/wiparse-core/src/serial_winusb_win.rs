//! Windows SetupAPI + WinUSB for CH-UART (ICD 1.1).
//!
//! Open `DeviceInterfaceGUID` `{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0101}` then
//! `WinUsb_Initialize`. CDC IAD: default iface is comm (EP 0x82); bulk 0x02/0x83
//! is `WinUsb_GetAssociatedInterface(0)`. `GLOBALROOT` is a fallback for
//! bcdDevice 0x0204 boards that omitted the UART GUID.

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::mem::{size_of, size_of_val, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceInterfaceDetailW,
    SetupDiGetDeviceRegistryPropertyW, DIGCF_ALLCLASSES, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    SPDRP_PHYSICAL_DEVICE_OBJECT_NAME, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
};
use windows_sys::Win32::Devices::Usb::{
    WinUsb_ControlTransfer, WinUsb_Free, WinUsb_GetAssociatedInterface, WinUsb_GetOverlappedResult,
    WinUsb_Initialize, WinUsb_ReadPipe, WinUsb_ResetPipe, WinUsb_SetPipePolicy, WinUsb_WritePipe,
    PIPE_TRANSFER_TIMEOUT, WINUSB_INTERFACE_HANDLE, WINUSB_SETUP_PACKET,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, FALSE, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, TRUE,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::OVERLAPPED;
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

const VIDPID: &str = "VID_C251&PID_F01A";
const UART_MI: u8 = 1;
const OUT_EP: u8 = 0x02;
const IN_EP: u8 = 0x83;
const CDC_SET_LINE_CODING: u8 = 0x20;
const CDC_SET_CONTROL_LINE_STATE: u8 = 0x22;

const GUID_UART: GUID = GUID {
    data1: 0x7B3E9C41,
    data2: 0x2A18,
    data3: 0x4F6D,
    data4: [0x9E, 0x50, 0x1C, 0x8A, 0x6D, 0x4B, 0x01, 0x01],
};

const GUID_I2C: GUID = GUID {
    data1: 0x7B3E9C41,
    data2: 0x2A18,
    data3: 0x4F6D,
    data4: [0x9E, 0x50, 0x1C, 0x8A, 0x6D, 0x4B, 0x01, 0x03],
};
const I2C_MI: u8 = 3;
const I2C_OUT_EP: u8 = 0x03;
const I2C_IN_EP: u8 = 0x84;
const I2C_READ_TIMEOUT_MS: u32 = 2_500;

pub struct NativeUart {
    file: HANDLE,
    /// CDC communication interface (line coding / DTR).
    winusb: WINUSB_INTERFACE_HANDLE,
    /// CDC data interface (bulk 0x02/0x83), or `winusb` if none.
    pipes: WINUSB_INTERFACE_HANDLE,
}

unsafe impl Send for NativeUart {}

impl NativeUart {
    pub fn open() -> Result<Self, String> {
        let mut last = "device path not found".to_string();
        let mut last_guid: Option<String> = None;
        let mut last_pdo: Option<String> = None;
        for path in candidate_paths(GUID_UART, UART_MI, "7b3e9c41-2a18-4f6d-9e50-1c8a6d4b0101") {
            match try_open_path(&path, OUT_EP, IN_EP) {
                Ok((file, winusb, pipes)) => {
                    return Ok(Self {
                        file,
                        winusb,
                        pipes,
                    });
                }
                Err(e) => {
                    let u = path.to_ascii_lowercase();
                    if u.contains("7b3e9c41") {
                        last_guid = Some(e.clone());
                    } else if u.contains("globalroot") {
                        last_pdo = Some(e.clone());
                    }
                    last = e;
                }
            }
        }
        Err(last_guid.or(last_pdo).unwrap_or(last))
    }

    pub fn apply_line_coding(&self, baud: u32) {
        let baud = baud.clamp(1_200, 2_000_000);
        let mut coding = [0u8; 7];
        coding[0..4].copy_from_slice(&baud.to_le_bytes());
        coding[4] = 0;
        coding[5] = 0;
        coding[6] = 8;
        let _ = self.control(0x21, CDC_SET_LINE_CODING, 0, UART_MI as u16, &coding, 2000);
        let _ = self.control(0x21, CDC_SET_CONTROL_LINE_STATE, 1, UART_MI as u16, &[], 2000);
    }

    fn set_dtr(&self, on: bool) {
        let v = if on { 1 } else { 0 };
        let _ = self.control(0x21, CDC_SET_CONTROL_LINE_STATE, v, UART_MI as u16, &[], 2000);
    }

    fn control(
        &self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
        timeout_ms: u32,
    ) -> Result<usize, io::Error> {
        let mut buf = data.to_vec();
        let pkt = WINUSB_SETUP_PACKET {
            RequestType: request_type,
            Request: request,
            Value: value,
            Index: index,
            Length: buf.len() as u16,
        };
        sync_xfer(self.winusb, None, timeout_ms, |ov| unsafe {
            let mut n = 0;
            let ptr = if buf.is_empty() {
                null_mut()
            } else {
                buf.as_mut_ptr()
            };
            let ok = WinUsb_ControlTransfer(
                self.winusb,
                pkt,
                ptr,
                buf.len() as u32,
                &mut n,
                ov,
            );
            (ok, n as usize)
        })
    }
}

impl Drop for NativeUart {
    fn drop(&mut self) {
        self.set_dtr(false);
        unsafe {
            if self.pipes != 0 && self.pipes != self.winusb {
                let _ = WinUsb_Free(self.pipes);
            }
            self.pipes = 0;
            if self.winusb != 0 {
                let _ = WinUsb_Free(self.winusb);
                self.winusb = 0;
            }
            if self.file != 0 && self.file != INVALID_HANDLE_VALUE {
                let _ = CloseHandle(self.file);
                self.file = INVALID_HANDLE_VALUE;
            }
        }
    }
}

impl Read for NativeUart {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(4096);
        match sync_xfer(self.pipes, Some(IN_EP), 50, |ov| unsafe {
            let mut transferred = 0;
            let ok = WinUsb_ReadPipe(
                self.pipes,
                IN_EP,
                buf.as_mut_ptr(),
                n as u32,
                &mut transferred,
                ov,
            );
            (ok, transferred as usize)
        }) {
            Ok(got) => Ok(got.min(buf.len())),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                Err(io::Error::new(io::ErrorKind::TimedOut, "timeout"))
            }
            Err(e) => Err(e),
        }
    }
}

pub struct NativeI2c {
    file: HANDLE,
    winusb: WINUSB_INTERFACE_HANDLE,
    pipes: WINUSB_INTERFACE_HANDLE,
}

unsafe impl Send for NativeI2c {}

impl NativeI2c {
    pub fn open() -> Result<Self, String> {
        let mut last = "device path not found".to_string();
        let mut last_guid: Option<String> = None;
        for path in candidate_paths(GUID_I2C, I2C_MI, "7b3e9c41-2a18-4f6d-9e50-1c8a6d4b0103") {
            match try_open_path(&path, I2C_OUT_EP, I2C_IN_EP) {
                Ok((file, winusb, pipes)) => {
                    return Ok(Self {
                        file,
                        winusb,
                        pipes,
                    });
                }
                Err(e) => {
                    if path.to_ascii_lowercase().contains("7b3e9c41") {
                        last_guid = Some(e.clone());
                    }
                    last = e;
                }
            }
        }
        Err(last_guid.unwrap_or(last))
    }
}

impl Drop for NativeI2c {
    fn drop(&mut self) {
        unsafe {
            if self.pipes != 0 && self.pipes != self.winusb {
                let _ = WinUsb_Free(self.pipes);
            }
            self.pipes = 0;
            if self.winusb != 0 {
                let _ = WinUsb_Free(self.winusb);
                self.winusb = 0;
            }
            if self.file != 0 && self.file != INVALID_HANDLE_VALUE {
                let _ = CloseHandle(self.file);
                self.file = INVALID_HANDLE_VALUE;
            }
        }
    }
}

impl Read for NativeI2c {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(4096);
        match sync_xfer(self.pipes, Some(I2C_IN_EP), I2C_READ_TIMEOUT_MS, |ov| unsafe {
            let mut transferred = 0;
            let ok = WinUsb_ReadPipe(
                self.pipes,
                I2C_IN_EP,
                buf.as_mut_ptr(),
                n as u32,
                &mut transferred,
                ov,
            );
            (ok, transferred as usize)
        }) {
            Ok(got) => Ok(got.min(buf.len())),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                Err(io::Error::new(io::ErrorKind::TimedOut, "timeout"))
            }
            Err(e) => Err(e),
        }
    }
}

impl Write for NativeI2c {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        sync_xfer(self.pipes, Some(I2C_OUT_EP), 2000, |ov| unsafe {
            let mut transferred = 0;
            let ok = WinUsb_WritePipe(
                self.pipes,
                I2C_OUT_EP,
                buf.as_ptr(),
                buf.len() as u32,
                &mut transferred,
                ov,
            );
            (ok, transferred as usize)
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Write for NativeUart {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        sync_xfer(self.pipes, Some(OUT_EP), 2000, |ov| unsafe {
            let mut transferred = 0;
            let ok = WinUsb_WritePipe(
                self.pipes,
                OUT_EP,
                buf.as_ptr(),
                buf.len() as u32,
                &mut transferred,
                ov,
            );
            (ok, transferred as usize)
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn try_open_path(
    path: &str,
    out_ep: u8,
    in_ep: u8,
) -> Result<(HANDLE, WINUSB_INTERFACE_HANDLE, WINUSB_INTERFACE_HANDLE), String> {
    let wide = wide(path);
    let file = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
            0,
        )
    };
    if file == 0 || file == INVALID_HANDLE_VALUE {
        return Err(format!("CreateFile {path} err={}", last_err()));
    }
    let mut winusb: WINUSB_INTERFACE_HANDLE = 0;
    let ok = unsafe { WinUsb_Initialize(file, &mut winusb) };
    if ok == FALSE || winusb == 0 {
        let e = last_err();
        unsafe {
            CloseHandle(file);
        }
        return Err(format!("WinUsb_Initialize {path} err={e}"));
    }
    let pipes = associated_pipes(winusb);
    if pipes != winusb {
        unsafe {
            let _ = WinUsb_ResetPipe(pipes, out_ep);
            let _ = WinUsb_ResetPipe(pipes, in_ep);
        }
    }
    Ok((file, winusb, pipes))
}

fn associated_pipes(winusb: WINUSB_INTERFACE_HANDLE) -> WINUSB_INTERFACE_HANDLE {
    let mut pipes: WINUSB_INTERFACE_HANDLE = 0;
    let ok = unsafe { WinUsb_GetAssociatedInterface(winusb, 0, &mut pipes) };
    if ok == FALSE || pipes == 0 {
        winusb
    } else {
        pipes
    }
}

fn candidate_paths(guid: GUID, mi: u8, guid_suffix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let push = |out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>, p: String| {
        if p.is_empty() {
            return;
        }
        if seen.insert(p.to_ascii_lowercase()) {
            out.push(p);
        }
    };
    for p in enumerate_interfaces(guid) {
        let u = p.to_ascii_uppercase();
        if u.contains(VIDPID) {
            push(&mut out, &mut seen, p);
        }
    }
    for inst in enumerate_usb_instances() {
        let u = inst.id.to_ascii_uppercase();
        if !u.contains(VIDPID) || parse_mi(&u) != Some(mi) {
            continue;
        }
        for p in instance_candidate_paths(&inst, guid_suffix) {
            push(&mut out, &mut seen, p);
        }
    }
    out
}

pub fn uart_function_present() -> bool {
    enumerate_usb_instances()
        .iter()
        .any(|inst| parse_mi(&inst.id.to_ascii_uppercase()) == Some(UART_MI))
}

struct UsbInst {
    id: String,
    pdo: String,
}

fn enumerate_usb_instances() -> Vec<UsbInst> {
    let mut out = Vec::new();
    let usb = wide("USB");
    let h = unsafe {
        SetupDiGetClassDevsW(
            null(),
            usb.as_ptr(),
            0,
            DIGCF_PRESENT | DIGCF_ALLCLASSES,
        )
    };
    if h == 0 || h == INVALID_HANDLE_VALUE {
        return out;
    }
    unsafe {
        for i in 0..512u32 {
            let mut info: SP_DEVINFO_DATA = zeroed();
            info.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
            if SetupDiEnumDeviceInfo(h, i, &mut info) == FALSE {
                break;
            }
            let mut buf = [0u16; 1024];
            if SetupDiGetDeviceInstanceIdW(h, &mut info, buf.as_mut_ptr(), buf.len() as u32, null_mut())
                == FALSE
            {
                continue;
            }
            let id = utf16_z(&buf);
            if !id.to_ascii_uppercase().contains(VIDPID) {
                continue;
            }
            let pdo = device_prop(h, &info, SPDRP_PHYSICAL_DEVICE_OBJECT_NAME);
            out.push(UsbInst { id, pdo });
        }
        SetupDiDestroyDeviceInfoList(h);
    }
    out
}

fn device_prop(h: isize, info: &SP_DEVINFO_DATA, prop: u32) -> String {
    let mut buf = [0u8; 1024];
    let mut ty = 0u32;
    let mut needed = 0u32;
    let ok = unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            h,
            info,
            prop,
            &mut ty,
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut needed,
        )
    };
    if ok == FALSE || needed < 2 {
        return String::new();
    }
    let n = (needed as usize / 2).min(buf.len() / 2);
    let u16s = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u16, n) };
    utf16_z(u16s)
}

fn enumerate_interfaces(guid: GUID) -> Vec<String> {
    let mut out = Vec::new();
    let mut g = guid;
    let h = unsafe { SetupDiGetClassDevsW(&mut g, null(), 0, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) };
    if h == 0 || h == INVALID_HANDLE_VALUE {
        return out;
    }
    unsafe {
        for i in 0..256u32 {
            let mut data: SP_DEVICE_INTERFACE_DATA = zeroed();
            data.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(h, null(), &g, i, &mut data) == FALSE {
                break;
            }
            let mut required = 0u32;
            SetupDiGetDeviceInterfaceDetailW(h, &mut data, null_mut(), 0, &mut required, null_mut());
            if required < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 {
                continue;
            }
            let mut buf = vec![0u8; required as usize];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = if size_of::<usize>() == 8 { 8 } else { 6 };
            if SetupDiGetDeviceInterfaceDetailW(
                h,
                &mut data,
                detail,
                required,
                &mut required,
                null_mut(),
            ) == FALSE
            {
                continue;
            }
            let path = utf16_z(std::slice::from_raw_parts(
                (*detail).DevicePath.as_ptr(),
                (required as usize / 2).saturating_sub(2).max(1),
            ));
            if !path.is_empty() {
                out.push(path);
            }
        }
        SetupDiDestroyDeviceInfoList(h);
    }
    out
}

fn instance_candidate_paths(inst: &UsbInst, guid_suffix: &str) -> Vec<String> {
    let hash = inst.id.replace('\\', "#");
    let body = if hash.starts_with(r"\\?\") {
        hash.clone()
    } else {
        format!(r"\\?\{hash}")
    };
    let mut out = vec![format!("{body}#{{{guid_suffix}}}")];
    out.extend(pdo_paths(&inst.pdo));
    out
}

fn pdo_paths(pdo: &str) -> Vec<String> {
    let pdo = pdo.trim_end_matches('\0').trim();
    if pdo.is_empty() {
        return Vec::new();
    }
    vec![
        format!(r"\\.\GLOBALROOT{pdo}"),
        format!(r"\\?\GLOBALROOT{pdo}"),
    ]
}

fn parse_mi(id_upper: &str) -> Option<u8> {
    let rest = id_upper.split("MI_").nth(1)?;
    u8::from_str_radix(rest.get(..2)?, 16).ok()
}

fn sync_xfer<F>(
    winusb: WINUSB_INTERFACE_HANDLE,
    pipe: Option<u8>,
    timeout_ms: u32,
    mut submit: F,
) -> io::Result<usize>
where
    F: FnMut(*mut OVERLAPPED) -> (i32, usize),
{
    unsafe {
        if let Some(pipe) = pipe {
            let mut timeout = timeout_ms.max(1);
            let _ = WinUsb_SetPipePolicy(
                winusb,
                pipe,
                PIPE_TRANSFER_TIMEOUT,
                size_of_val(&timeout) as u32,
                &mut timeout as *mut u32 as *mut _,
            );
        }
        let ev = CreateEventW(null(), TRUE, FALSE, null());
        if ev == 0 || ev == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut ov: OVERLAPPED = zeroed();
        ov.hEvent = ev;
        let (ok, n) = submit(&mut ov);
        if ok != FALSE {
            CloseHandle(ev);
            return Ok(n);
        }
        let err = last_err();
        if err != 997 {
            CloseHandle(ev);
            if err == 995 || err == 121 {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "timeout"));
            }
            return Err(io::Error::from_raw_os_error(err));
        }
        let wr = WaitForSingleObject(ev, timeout_ms.max(1));
        if wr != WAIT_OBJECT_0 {
            CloseHandle(ev);
            return Err(io::Error::new(io::ErrorKind::TimedOut, "pipe timeout"));
        }
        let mut transferred = 0u32;
        let ok = WinUsb_GetOverlappedResult(winusb, &ov, &mut transferred, FALSE);
        CloseHandle(ev);
        if ok == FALSE {
            let e = last_err();
            if e == 995 || e == 121 {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "timeout"));
            }
            return Err(io::Error::from_raw_os_error(e));
        }
        Ok(transferred as usize)
    }
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn utf16_z(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

fn last_err() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mi_from_instance_id() {
        assert_eq!(
            parse_mi(r"USB\VID_C251&PID_F01A&MI_01\8&12C8C065&0&0001"),
            Some(1)
        );
        assert_eq!(
            parse_mi(r"USB\VID_C251&PID_F01A&MI_00\X"),
            Some(0)
        );
        assert_eq!(parse_mi(r"USB\VID_C251&PID_F01A\348518980FAC"), None);
    }

    #[test]
    fn instance_paths_include_winusb_suffix() {
        let p = instance_candidate_paths(
            &UsbInst {
                id: r"USB\VID_C251&PID_F01A&MI_01\ABC".into(),
                pdo: r"\Device\000004a9".into(),
            },
            "7b3e9c41-2a18-4f6d-9e50-1c8a6d4b0101",
        );
        assert!(p[0].contains("MI_01") && p[0].contains("7b3e9c41"));
        let i2c = instance_candidate_paths(
            &UsbInst {
                id: r"USB\VID_C251&PID_F01A&MI_03\ABC".into(),
                pdo: r"\Device\000004cb".into(),
            },
            "7b3e9c41-2a18-4f6d-9e50-1c8a6d4b0103",
        );
        assert!(i2c[0].contains("MI_03") && i2c[0].contains("0103"));
        assert!(p.iter().any(|s| s == r"\\.\GLOBALROOT\Device\000004a9"));
    }

    #[test]
    fn pdo_paths_prefix_globalroot() {
        assert_eq!(
            pdo_paths(r"\Device\000004a9"),
            vec![
                r"\\.\GLOBALROOT\Device\000004a9".to_string(),
                r"\\?\GLOBALROOT\Device\000004a9".to_string(),
            ]
        );
        assert!(pdo_paths("").is_empty());
    }
}
