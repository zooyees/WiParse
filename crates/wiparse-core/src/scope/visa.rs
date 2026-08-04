//! Runtime NI-VISA / TekVISA bindings via `visa64.dll` / `visa32.dll` (no link-time dep).

use libloading::Library;
use std::ffi::{c_char, c_void, CStr, CString};
use std::io::{self, Read, Write};
use std::ptr;
use std::sync::Arc;
use thiserror::Error;

pub type ViStatus = i32;
pub type ViSession = u32;
pub type ViObject = u32;
pub type ViUInt32 = u32;
pub type ViFindList = u32;
pub type ViAttr = u32;
pub type ViAttrState = u64;

pub const VI_SUCCESS: ViStatus = 0;
pub const VI_NULL: u32 = 0;
pub const VI_ATTR_TMO_VALUE: ViAttr = 0x3FFF_0015;
const VI_NO_LOCK: u32 = 0;
const VI_TMO_IMMEDIATE: u32 = 0;
const VI_ERROR_RSRC_NFOUND: ViStatus = -1073807343;
const VI_ERROR_INV_OBJECT: ViStatus = -1073807346;
const VI_ERROR_TMO: ViStatus = -1073807339;
const VI_ERROR_NSUP_ATTR: ViStatus = -1073807331;
const VI_ERROR_NSUP_ATTR_STATE: ViStatus = -1073807330;
const VI_FIND_BUFLEN: usize = 256;

fn decode_status_desc(raw: &[u8]) -> String {
    if raw.is_empty() {
        return String::new();
    }
    if let Ok(s) = std::str::from_utf8(raw) {
        if s.is_ascii() || s.chars().all(|c| !c.is_control()) {
            return s.to_string();
        }
    }
    #[cfg(windows)]
    {
        if let Some(s) = decode_acp(raw) {
            return s;
        }
    }
    String::from_utf8_lossy(raw).into_owned()
}

#[cfg(windows)]
fn decode_acp(raw: &[u8]) -> Option<String> {
    use std::ptr;
    type WCHAR = u16;
    extern "system" {
        fn MultiByteToWideChar(
            code_page: u32,
            flags: u32,
            bytes: *const u8,
            nbytes: i32,
            wide: *mut WCHAR,
            wide_len: i32,
        ) -> i32;
    }
    const CP_ACP: u32 = 0;
    let need = unsafe {
        MultiByteToWideChar(
            CP_ACP,
            0,
            raw.as_ptr(),
            raw.len() as i32,
            ptr::null_mut(),
            0,
        )
    };
    if need <= 0 {
        return None;
    }
    let mut wide = vec![0u16; need as usize];
    let written = unsafe {
        MultiByteToWideChar(
            CP_ACP,
            0,
            raw.as_ptr(),
            raw.len() as i32,
            wide.as_mut_ptr(),
            need,
        )
    };
    if written <= 0 {
        return None;
    }
    wide.truncate(written as usize);
    String::from_utf16(&wide).ok()
}

fn status_label(status: ViStatus) -> &'static str {
    match status {
        VI_ERROR_RSRC_NFOUND => "VI_ERROR_RSRC_NFOUND",
        VI_ERROR_INV_OBJECT => "VI_ERROR_INV_OBJECT",
        VI_ERROR_TMO => "VI_ERROR_TMO",
        VI_ERROR_NSUP_ATTR => "VI_ERROR_NSUP_ATTR",
        VI_ERROR_NSUP_ATTR_STATE => "VI_ERROR_NSUP_ATTR_STATE",
        _ => "VI_ERROR",
    }
}

#[derive(Debug, Error)]
pub enum VisaError {
    #[error("VISA library not found (install NI-VISA or TekVISA): {0}")]
    Load(String),
    #[error("VISA error 0x{0:08X}: {1}")]
    Status(u32, String),
    #[error("I/O: {0}")]
    Io(String),
}

type FnOpenDefaultRm = unsafe extern "system" fn(*mut ViSession) -> ViStatus;
type FnFindRsrc = unsafe extern "system" fn(
    ViSession,
    *const c_char,
    *mut ViFindList,
    *mut ViUInt32,
    *mut c_char,
) -> ViStatus;
type FnFindNext = unsafe extern "system" fn(ViFindList, *mut c_char) -> ViStatus;
type FnOpen =
    unsafe extern "system" fn(ViSession, *const c_char, u32, ViUInt32, *mut ViSession) -> ViStatus;
type FnClose = unsafe extern "system" fn(ViObject) -> ViStatus;
type FnWrite = unsafe extern "system" fn(ViSession, *const u8, ViUInt32, *mut ViUInt32) -> ViStatus;
type FnRead = unsafe extern "system" fn(ViSession, *mut u8, ViUInt32, *mut ViUInt32) -> ViStatus;
type FnSetAttribute = unsafe extern "system" fn(ViObject, ViAttr, ViAttrState) -> ViStatus;
type FnGetAttribute = unsafe extern "system" fn(ViObject, ViAttr, *mut ViAttrState) -> ViStatus;
type FnClear = unsafe extern "system" fn(ViSession) -> ViStatus;
type FnStatusDesc = unsafe extern "system" fn(ViObject, ViStatus, *mut c_char) -> ViStatus;

struct VisaApi {
    _lib: Library,
    source: String,
    open_default_rm: FnOpenDefaultRm,
    find_rsrc: FnFindRsrc,
    find_next: FnFindNext,
    open: FnOpen,
    close: FnClose,
    write: FnWrite,
    read: FnRead,
    set_attribute: FnSetAttribute,
    get_attribute: Option<FnGetAttribute>,
    clear: Option<FnClear>,
    status_desc: Option<FnStatusDesc>,
}

impl VisaApi {
    fn load(preferred: Option<&str>) -> Result<Arc<Self>, VisaError> {
        let mut tried = Vec::new();
        // Match Python PyVISA on this machine (visa32.dll in System32).
        // Prefer PYVISA_LIBRARY when set.
        let mut names: Vec<String> = Vec::new();
        if let Some(preferred) = preferred.filter(|value| !value.trim().is_empty()) {
            names.push(preferred.to_owned());
        }
        if let Ok(env) = std::env::var("PYVISA_LIBRARY") {
            if !env.is_empty() {
                names.push(env);
            }
        }
        names.extend(
            [
                "visa32.dll",
                "visa64.dll",
                "nivisa64.dll",
                "tekvisa64.dll",
                r"C:\Windows\System32\visa32.dll",
                r"C:\Windows\System32\visa64.dll",
                r"C:\Windows\SysWOW64\visa32.dll",
                r"C:\Program Files\IVI Foundation\VISA\Win64\Bin\visa64.dll",
                r"C:\Program Files\IVI Foundation\VISA\Win64\Bin\visa32.dll",
                r"C:\Program Files (x86)\IVI Foundation\VISA\Win64\Bin\visa64.dll",
                r"C:\Program Files\TekVISA\Bin\visa64.dll",
                r"C:\Program Files\TekVISA\Bin\visa32.dll",
            ]
            .into_iter()
            .map(str::to_string),
        );

        for name in &names {
            match unsafe { Library::new(name) } {
                Ok(lib) => match unsafe { Self::from_lib(lib, name.clone()) } {
                    Ok(api) => {
                        tracing::info!("Loaded VISA library: {name}");
                        return Ok(Arc::new(api));
                    }
                    Err(e) => tried.push(format!("{name}: {e}")),
                },
                Err(e) => tried.push(format!("{name}: {e}")),
            }
        }
        Err(VisaError::Load(tried.join(" | ")))
    }

    unsafe fn from_lib(lib: Library, source: String) -> Result<Self, VisaError> {
        unsafe {
            let open_default_rm = *lib
                .get::<FnOpenDefaultRm>(b"viOpenDefaultRM\0")
                .map_err(|e| VisaError::Load(e.to_string()))?;
            let find_rsrc = *lib
                .get::<FnFindRsrc>(b"viFindRsrc\0")
                .map_err(|e| VisaError::Load(e.to_string()))?;
            let find_next = *lib
                .get::<FnFindNext>(b"viFindNext\0")
                .map_err(|e| VisaError::Load(e.to_string()))?;
            let open = *lib
                .get::<FnOpen>(b"viOpen\0")
                .map_err(|e| VisaError::Load(e.to_string()))?;
            let close = *lib
                .get::<FnClose>(b"viClose\0")
                .map_err(|e| VisaError::Load(e.to_string()))?;
            let write = *lib
                .get::<FnWrite>(b"viWrite\0")
                .map_err(|e| VisaError::Load(e.to_string()))?;
            let read = *lib
                .get::<FnRead>(b"viRead\0")
                .map_err(|e| VisaError::Load(e.to_string()))?;
            let set_attribute = *lib
                .get::<FnSetAttribute>(b"viSetAttribute\0")
                .map_err(|e| VisaError::Load(e.to_string()))?;
            let get_attribute = lib
                .get::<FnGetAttribute>(b"viGetAttribute\0")
                .ok()
                .map(|s| *s);
            let clear = lib.get::<FnClear>(b"viClear\0").ok().map(|s| *s);
            let status_desc = lib.get::<FnStatusDesc>(b"viStatusDesc\0").ok().map(|s| *s);
            let _ = ptr::null::<c_void>();
            Ok(Self {
                _lib: lib,
                source,
                open_default_rm,
                find_rsrc,
                find_next,
                open,
                close,
                write,
                read,
                set_attribute,
                get_attribute,
                clear,
                status_desc,
            })
        }
    }

    fn check(&self, session: ViObject, status: ViStatus) -> Result<(), VisaError> {
        // VISA: 0 = success, >0 = warning (still OK).
        if status >= VI_SUCCESS {
            return Ok(());
        }
        let mut buf = [0i8; 256];
        if let Some(desc) = self.status_desc {
            unsafe {
                desc(session, status, buf.as_mut_ptr());
            }
        }
        let raw = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_bytes();
        let desc = decode_status_desc(raw);
        let msg = if desc.is_empty() {
            status_label(status).to_string()
        } else {
            format!("{} — {desc}", status_label(status))
        };
        Err(VisaError::Status(status as u32, msg))
    }
}

pub struct ResourceManager {
    api: Arc<VisaApi>,
    session: ViSession,
}

impl ResourceManager {
    pub fn new() -> Result<Self, VisaError> {
        Self::new_with_library(None)
    }

    pub fn new_with_library(preferred: Option<&str>) -> Result<Self, VisaError> {
        let api = VisaApi::load(preferred)?;
        let mut session = 0;
        let st = unsafe { (api.open_default_rm)(&mut session) };
        api.check(VI_NULL, st)?;
        Ok(Self { api, session })
    }

    pub fn library_source(&self) -> &str {
        &self.api.source
    }

    pub fn list_resources(&self, expr: &str) -> Result<Vec<String>, VisaError> {
        let c_expr = CString::new(expr).map_err(|e| VisaError::Io(e.to_string()))?;
        let mut find_list = 0;
        let mut count = 0u32;
        let mut buf = [0i8; VI_FIND_BUFLEN];
        let st = unsafe {
            (self.api.find_rsrc)(
                self.session,
                c_expr.as_ptr(),
                &mut find_list,
                &mut count,
                buf.as_mut_ptr(),
            )
        };
        if st < 0 {
            // No matches often returns error — treat as empty
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        if count > 0 {
            out.push(unsafe { CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned() });
            for _ in 1..count {
                let mut next = [0i8; VI_FIND_BUFLEN];
                let st = unsafe { (self.api.find_next)(find_list, next.as_mut_ptr()) };
                if st < 0 {
                    break;
                }
                out.push(unsafe { CStr::from_ptr(next.as_ptr()).to_string_lossy().into_owned() });
            }
        }
        let _ = unsafe { (self.api.close)(find_list) };
        Ok(out)
    }

    pub fn open(&self, resource: &str, timeout_ms: u32) -> Result<Instrument, VisaError> {
        let c_res = CString::new(resource).map_err(|e| VisaError::Io(e.to_string()))?;
        let mut instr = 0u32;
        // Match PyVISA: with VI_NO_LOCK, openTimeout is unused — pass VI_TMO_IMMEDIATE.
        // (Passing the I/O timeout here led some stacks to return confusing attribute errors.)
        let st = unsafe {
            (self.api.open)(
                self.session,
                c_res.as_ptr(),
                VI_NO_LOCK,
                VI_TMO_IMMEDIATE,
                &mut instr,
            )
        };
        self.api
            .check(self.session, st)
            .map_err(|e| VisaError::Io(format!("viOpen({resource}) failed: {e}")))?;
        if instr == 0 {
            return Err(VisaError::Io(format!(
                "viOpen({resource}) returned null session"
            )));
        }
        let inst = Instrument {
            api: Arc::clone(&self.api),
            session: instr,
        };
        // Timeout is optional on some USB/TekVISA sessions (0xBFFF001D NSUP_ATTR).
        // Python still communicates if attribute set fails; do not abort the open.
        if let Err(e) = inst.set_timeout(timeout_ms) {
            tracing::warn!("viSetAttribute(TMO) ignored for {resource}: {e}");
        }
        Ok(inst)
    }
}

impl Drop for ResourceManager {
    fn drop(&mut self) {
        if self.session != 0 {
            unsafe {
                (self.api.close)(self.session);
            }
            self.session = 0;
        }
    }
}

pub struct Instrument {
    api: Arc<VisaApi>,
    session: ViSession,
}

impl Instrument {
    pub fn set_timeout(&self, timeout_ms: u32) -> Result<(), VisaError> {
        let st = unsafe {
            (self.api.set_attribute)(
                self.session,
                VI_ATTR_TMO_VALUE,
                ViAttrState::from(timeout_ms),
            )
        };
        if st == VI_ERROR_NSUP_ATTR || st == VI_ERROR_NSUP_ATTR_STATE {
            return Err(self
                .api
                .check(self.session, st)
                .err()
                .unwrap_or_else(|| VisaError::Status(st as u32, status_label(st).into())));
        }
        self.api.check(self.session, st)
    }

    pub fn timeout_ms(&self) -> Option<u32> {
        let get = self.api.get_attribute?;
        let mut state: ViAttrState = 0;
        let st = unsafe { get(self.session, VI_ATTR_TMO_VALUE, &mut state) };
        if st < 0 {
            return None;
        }
        Some(state as u32)
    }

    /// Device clear — flushes I/O buffers after aborted CURVe/HARDCopy transfers.
    pub fn clear(&self) -> Result<(), VisaError> {
        let Some(clear) = self.api.clear else {
            return Ok(());
        };
        let st = unsafe { clear(self.session) };
        self.api.check(self.session, st)
    }

    /// Drain leftover input with a short timeout (best-effort).
    pub fn discard_input(&self) {
        let prev = self.timeout_ms().unwrap_or(5_000);
        let _ = self.set_timeout(20);
        // Keep this tight — clear_io is used on the hot path before CURVe.
        for _ in 0..3 {
            match self.read_bytes(64 * 1024) {
                Ok(buf) if !buf.is_empty() => continue,
                _ => break,
            }
        }
        let _ = self.set_timeout(prev.max(100));
    }

    pub fn write_bytes(&self, data: &[u8]) -> Result<(), VisaError> {
        let mut written = 0;
        let st = unsafe {
            (self.api.write)(self.session, data.as_ptr(), data.len() as u32, &mut written)
        };
        self.api.check(self.session, st)
    }

    pub fn write_str(&self, cmd: &str) -> Result<(), VisaError> {
        let mut data = cmd.as_bytes().to_vec();
        if !data.ends_with(b"\n") {
            data.push(b'\n');
        }
        self.write_bytes(&data)
    }

    pub fn read_bytes(&self, max: usize) -> Result<Vec<u8>, VisaError> {
        let mut buf = vec![0u8; max];
        let mut ret = 0;
        let st = unsafe { (self.api.read)(self.session, buf.as_mut_ptr(), max as u32, &mut ret) };
        // PyVISA often returns VI_ERROR_TMO with retCount > 0 for terminated reads.
        if st < 0 {
            if ret > 0 && (st == VI_ERROR_TMO || st == VI_ERROR_RSRC_NFOUND) {
                buf.truncate(ret as usize);
                return Ok(buf);
            }
            if ret > 0 && st != VI_ERROR_INV_OBJECT {
                // Prefer bytes already transferred over a hard failure.
                buf.truncate(ret as usize);
                return Ok(buf);
            }
            return self.api.check(self.session, st).map(|_| unreachable!());
        }
        buf.truncate(ret as usize);
        Ok(buf)
    }

    pub fn query_str(&self, cmd: &str) -> Result<String, VisaError> {
        self.write_str(cmd)?;
        let raw = self.read_bytes(64 * 1024)?;
        Ok(String::from_utf8_lossy(&raw).trim().to_string())
    }

    /// Hard limit for HARDCopy / CURVe binary reads (prevents OOM on runaway instruments).
    /// 64 MiB ≈ 60M i8 samples — enough for 10M pt Tek CURVe with IEEE header.
    pub const MAX_RAW_READ_BYTES: usize = 64 * 1024 * 1024;

    pub fn read_raw(&self) -> Result<Vec<u8>, VisaError> {
        // HARDCopy / CURVe can be large — read in chunks with a hard ceiling.
        // Critical: a short/timeout chunk must NOT end the read while an IEEE
        // `#N…` block header says more payload bytes are still outstanding
        // (MDO3014 CURVe is multi‑MiB). But each empty/timeout retry costs a full
        // VISA timeout — keep the idle budget small for HARDCopy/PNG, larger for CURVe.
        use crate::scope::binary::{
            ieee_block_complete, ieee_block_header_offset, parse_ieee_block, png_complete,
        };

        const CHUNK: usize = 1024 * 1024;
        let mut all = Vec::new();
        let mut idle_errors = 0u32;
        loop {
            let done = |buf: &[u8]| -> bool {
                if ieee_block_complete(buf) || png_complete(buf) {
                    return true;
                }
                // HARDCopy: payload may be a finished PNG even if the outer length
                // prefix was truncated / slightly wrong — stop without more timeouts.
                let payload = parse_ieee_block(buf);
                png_complete(payload)
            };

            match self.read_bytes(CHUNK) {
                Ok(chunk) if chunk.is_empty() => {
                    if done(&all) || ieee_block_header_offset(&all).is_none() {
                        break;
                    }
                    idle_errors += 1;
                    // CURVe mid-transfer: allow more retries; HARDCopy should finish sooner.
                    let max_idle = if all.len() > 512 * 1024 { 48 } else { 6 };
                    if idle_errors >= max_idle {
                        break;
                    }
                }
                Ok(chunk) => {
                    idle_errors = 0;
                    if all.len().saturating_add(chunk.len()) > Self::MAX_RAW_READ_BYTES {
                        return Err(VisaError::Io(format!(
                            "binary read exceeded {} MiB limit",
                            Self::MAX_RAW_READ_BYTES / (1024 * 1024)
                        )));
                    }
                    all.extend_from_slice(&chunk);
                    if done(&all) {
                        break;
                    }
                    let framed = ieee_block_header_offset(&all).is_some();
                    // Short chunk = END for unframed / finished PNG; keep going only
                    // when a definite-length IEEE header still promises more bytes.
                    if chunk.len() < CHUNK && (!framed || done(&all)) {
                        break;
                    }
                    if chunk.len() < CHUNK && framed && !ieee_block_complete(&all) {
                        // Instrument paused mid-block — one more read cycle, but do not
                        // spin on timeouts forever (screenshot regression).
                        idle_errors += 1;
                        let max_idle = if all.len() > 512 * 1024 { 48 } else { 6 };
                        if idle_errors >= max_idle {
                            break;
                        }
                    }
                }
                Err(e) => {
                    if done(&all) {
                        break;
                    }
                    if all.is_empty() {
                        return Err(e);
                    }
                    idle_errors += 1;
                    let max_idle = if all.len() > 512 * 1024 { 48 } else { 6 };
                    if idle_errors >= max_idle {
                        break;
                    }
                }
            }
        }
        Ok(all)
    }
}

impl Drop for Instrument {
    fn drop(&mut self) {
        if self.session != 0 {
            unsafe {
                (self.api.close)(self.session);
            }
            self.session = 0;
        }
    }
}

impl Write for Instrument {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_bytes(buf)
            .map_err(|e| io::Error::other(e.to_string()))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for Instrument {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let data = self
            .read_bytes(buf.len())
            .map_err(|e| io::Error::other(e.to_string()))?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok(n)
    }
}
