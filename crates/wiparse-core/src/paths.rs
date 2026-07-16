//! Path helpers — writable root vs bundled assets.

use std::env;
use std::path::{Path, PathBuf};

/// Writable data root: directory containing the running executable, or cwd in tests/dev.
pub fn app_root() -> PathBuf {
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Prefer cargo target/.../deps parent chain only when running from target/
            let s = parent.to_string_lossy();
            if s.contains("target") && (s.contains("debug") || s.contains("release")) {
                // Climb to workspace root when running via `cargo run`
                if let Ok(manifest) = env::var("CARGO_MANIFEST_DIR") {
                    // crates/wiparse-*/ → workspace root
                    let p = PathBuf::from(manifest);
                    if let Some(ws) = p.parent().and_then(|p| p.parent()) {
                        return ws.to_path_buf();
                    }
                }
            }
            return parent.to_path_buf();
        }
    }
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn project_path(relative: impl AsRef<Path>) -> PathBuf {
    let p = relative.as_ref();
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        app_root().join(p)
    }
}

pub fn config_file() -> PathBuf {
    if let Ok(env_path) = env::var("WCM_CONFIG") {
        return PathBuf::from(env_path);
    }
    project_path("config.json")
}

pub fn default_config_file() -> PathBuf {
    project_path("config.default.json")
}

pub fn app_icon_path() -> Option<PathBuf> {
    let rel = Path::new("Icon").join("WiParse.ico");
    let candidate = project_path(&rel);
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_path_joins() {
        let p = project_path("config.json");
        assert!(p.ends_with("config.json"));
    }
}
