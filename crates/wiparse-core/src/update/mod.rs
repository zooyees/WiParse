//! Online update manifest, version compare, and package verification.
//!
//! HTTP download and install orchestration live in the GUI crate; measurement and
//! instrument data paths are unaffected by updates.

mod fetch;
mod manifest;
mod verify;
mod version;

pub use fetch::{fetch_manifest, FetchError};

pub use manifest::{UpdateManifest, UpdatePackage, UpdateTarget};
pub use verify::{hex_sha256, verify_file_sha256, VerifyError};
pub use version::{compare_versions, is_newer_version, parse_semver, VersionCmp};

use crate::config::UpdateConfig;

/// Result of comparing the running build against a remote manifest.
#[derive(Debug, Clone)]
pub enum UpdateAvailability {
    Disabled,
    UpToDate,
    UpdateAvailable {
        manifest: UpdateManifest,
        package: UpdatePackage,
    },
    BelowMinimum {
        current: String,
        min_version: String,
    },
}

impl UpdateAvailability {
    pub fn is_update_available(&self) -> bool {
        matches!(self, Self::UpdateAvailable { .. })
    }
}

/// Evaluate manifest against the running version (no network I/O).
pub fn evaluate_manifest(
    current_version: &str,
    manifest: &UpdateManifest,
    target: UpdateTarget,
) -> UpdateAvailability {
    if let Some(min) = manifest.min_version.as_deref() {
        if is_newer_version(min, current_version) {
            return UpdateAvailability::BelowMinimum {
                current: current_version.to_string(),
                min_version: min.to_string(),
            };
        }
    }
    if !is_newer_version(&manifest.version, current_version) {
        return UpdateAvailability::UpToDate;
    }
    let Some(package) = manifest.package_for(target) else {
        return UpdateAvailability::UpToDate;
    };
    UpdateAvailability::UpdateAvailable {
        manifest: manifest.clone(),
        package: package.clone(),
    }
}

/// Build the manifest URL from config (empty → updates disabled).
pub fn manifest_url(cfg: &UpdateConfig) -> Option<String> {
    if !cfg.enabled {
        return None;
    }
    let url = cfg.manifest_url.trim();
    if url.is_empty() {
        return None;
    }
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluate_finds_newer_package() {
        let manifest: UpdateManifest = serde_json::from_str(include_str!(
            "../../../../packaging/update/latest.json.example"
        ))
        .unwrap();
        let avail = evaluate_manifest("1.0.0", &manifest, UpdateTarget::WindowsX64);
        assert!(avail.is_update_available());
    }
}
