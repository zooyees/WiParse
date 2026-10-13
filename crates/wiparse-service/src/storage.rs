use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
pub struct RootLock {
    path: PathBuf,
}
impl RootLock {
    pub fn acquire(root: &Path) -> Result<Self, String> {
        let path = root.join("service.lock");
        let mut f=fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e|format!("SERVICE.ROOT_LOCKED ({e}); verify previous service is stopped before removing stale lock"))?;
        writeln!(f, "{}", std::process::id()).map_err(|e| e.to_string())?;
        Ok(Self { path })
    }
}
impl Drop for RootLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
pub fn atomic_json(path: &Path, value: &Value) -> Result<(), String> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut f = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(
        serde_json::to_string_pretty(value)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    let bak = path.with_extension("json.bak");
    if path.exists() {
        fs::copy(path, &bak).map_err(|e| e.to_string())?;
        fs::OpenOptions::new()
            .write(true)
            .open(&bak)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    if let Err(e) = replace(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    Ok(())
}
#[cfg(windows)]
fn replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{ReplaceFileW, REPLACEFILE_WRITE_THROUGH};
    if !destination.exists() {
        return fs::rename(source, destination);
    }
    let src: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let dst: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let ok = unsafe {
        ReplaceFileW(
            dst.as_ptr(),
            src.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}
pub fn safe_input(root: &Path, raw: &str) -> Result<PathBuf, String> {
    let p = Path::new(raw);
    let path = fs::canonicalize(if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    })
    .map_err(|e| e.to_string())?;
    let projects =
        fs::canonicalize(root.join("projects")).map_err(|_| "DATA.PROJECT_ROOT_REQUIRED")?;
    if !path.starts_with(&projects) || !path.is_file() {
        return Err("DATA.PATH_OUTSIDE_PROJECT".into());
    }
    Ok(path)
}
pub fn file_hash(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut b = [0u8; 65536];
    use std::io::Read;
    loop {
        let n = f.read(&mut b).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
