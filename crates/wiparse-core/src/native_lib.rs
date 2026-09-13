//! Locate optional native libraries next to the exe (portable dist) before PATH.

use libloading::Library;
use std::env;
use std::path::{Path, PathBuf};

/// Load the first library name that exists in the WiParse search path.
pub fn load_named(names: &[&str]) -> Result<Library, libloading::Error> {
    let mut last = None;
    for dir in search_dirs() {
        for name in names {
            let path = dir.join(name);
            if !path.is_file() {
                continue;
            }
            match unsafe { Library::new(&path) } {
                Ok(lib) => {
                    tracing::info!("loaded native library {}", path.display());
                    return Ok(lib);
                }
                Err(e) => last = Some(e),
            }
        }
    }
    for name in names {
        match unsafe { Library::new(name) } {
            Ok(lib) => return Ok(lib),
            Err(e) => last = Some(e),
        }
    }
    if let Some(e) = last {
        return Err(e);
    }
    unsafe { Library::new(names.first().copied().unwrap_or("missing.dll")) }
}

/// First existing path for any of `names`, for status text (does not load).
pub fn find_named(names: &[&str]) -> Option<PathBuf> {
    for dir in search_dirs() {
        for name in names {
            let path = dir.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

pub fn ftdi_dll_names() -> &'static [&'static str] {
    &[
        "LibFT4222.dll",
        "LibFT4222-64.dll",
        "libft4222.dll",
        "libft4222-64.dll",
    ]
}

pub fn ftd2xx_dll_names() -> &'static [&'static str] {
    &["ftd2xx.dll", "FTD2XX.dll", "FTD2XX64.dll"]
}

pub fn ftdi_runtime_report() -> String {
    let ft = find_named(ftdi_dll_names());
    let d2 = find_named(ftd2xx_dll_names());
    match (ft, d2) {
        (Some(a), Some(b)) => format!(
            "FT4222 runtime bundled/found:\n  {}\n  {}",
            a.display(),
            b.display()
        ),
        (None, None) => {
            "LibFT4222.dll / ftd2xx.dll not next to WiParse.exe. Drop them in vendor/ftdi or install the FTDI FT4222 package (SPI/I2C/GPIO needs them; scan does not)."
                .into()
        }
        (Some(a), None) => format!(
            "Found {} but missing ftd2xx.dll — copy both DLLs into the WiParse folder or vendor/ftdi.",
            a.display()
        ),
        (None, Some(b)) => format!(
            "Found {} but missing LibFT4222.dll — copy both DLLs into the WiParse folder or vendor/ftdi.",
            b.display()
        ),
    }
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let push = |dirs: &mut Vec<PathBuf>, p: PathBuf| {
        if p.is_dir() && !dirs.iter().any(|d| d == &p) {
            dirs.push(p);
        }
    };

    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            push(&mut dirs, parent.to_path_buf());
            push(&mut dirs, parent.join("vendor").join("ftdi"));
            push(&mut dirs, parent.join("ftdi"));
        }
    }

    let root = crate::paths::app_root();
    push(&mut dirs, root.join("vendor").join("ftdi"));
    push(&mut dirs, root.join("dist").join("vendor").join("ftdi"));

    for key in ["WIPARSE_FTDI_DIR", "WIPARSE_NATIVE_DIR"] {
        if let Ok(v) = env::var(key) {
            let t = v.trim();
            if !t.is_empty() {
                push(&mut dirs, PathBuf::from(t));
            }
        }
    }

    for extra in ftdi_install_dirs() {
        push(&mut dirs, extra);
    }

    if let Ok(sys) = env::var("SystemRoot") {
        push(&mut dirs, Path::new(&sys).join("System32"));
    }

    dirs
}

fn ftdi_install_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let pf = env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    let pf86 = env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
    for root in [pf, pf86] {
        let base = PathBuf::from(root).join("FTDI");
        for rel in [
            "LibFT4222\\imports\\LibFT4222\\dll\\amd64",
            "LibFT4222\\dll\\amd64",
            "LibFT4222\\amd64",
            "FT4222\\amd64",
            "FT4222H\\amd64",
            "D2XX\\amd64",
        ] {
            out.push(base.join(rel));
        }
    }
    out
}
