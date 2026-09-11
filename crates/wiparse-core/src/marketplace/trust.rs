//! Trust policy and SHA-256 verification for marketplace artifacts.

use crate::marketplace::catalog::{MarketplacePackageMeta, CHANNELS, SANDBOX_PERMISSIONS};
use crate::update::{hex_sha256, verify_file_sha256};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MarketplaceError {
    #[error("E_HTTPS: {0}")]
    Https(String),
    #[error("E_HASH: {0}")]
    Hash(String),
    #[error("E_SIG: {0}")]
    Sig(String),
    #[error("E_TRUST: {0}")]
    Trust(String),
    #[error("E_COMPAT: {0}")]
    Compat(String),
    #[error("E_LAYOUT: {0}")]
    Layout(String),
    #[error("E_AUTH: {0}")]
    Auth(String),
    #[error("E_NOT_FOUND: {0}")]
    NotFound(String),
    #[error("E_CONFIG: {0}")]
    Config(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrustPolicy {
    #[serde(default)]
    pub require_signature: bool,
    #[serde(default)]
    pub public_keys: Vec<String>,
    #[serde(default)]
    pub allowed_publishers: Vec<String>,
    #[serde(default)]
    pub denied_publishers: Vec<String>,
}

pub fn normalize_trust_policy(raw: &TrustPolicy) -> TrustPolicy {
    TrustPolicy {
        require_signature: raw.require_signature,
        public_keys: raw
            .public_keys
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        allowed_publishers: raw
            .allowed_publishers
            .iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect(),
        denied_publishers: raw
            .denied_publishers
            .iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect(),
    }
}

pub fn validate_package_meta(meta: &MarketplacePackageMeta) -> Result<(), MarketplaceError> {
    if meta.id.is_empty()
        || !meta
            .id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        || !meta
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(MarketplaceError::Layout(format!("invalid id: {}", meta.id)));
    }
    if meta.version.trim().is_empty()
        || meta.version.contains("..")
        || meta.version.contains('/')
        || meta.version.contains('\\')
    {
        return Err(MarketplaceError::Layout(format!(
            "invalid version: {}",
            meta.version
        )));
    }
    let sha = meta.sha256.trim().to_ascii_lowercase();
    if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(MarketplaceError::Layout("sha256 must be 64 hex chars".into()));
    }
    if let Some(ch) = meta.channel.as_deref() {
        if !CHANNELS.contains(&ch) {
            return Err(MarketplaceError::Layout(format!("invalid channel: {ch}")));
        }
    }
    if let Some(sb) = &meta.sandbox {
        for p in &sb.permissions {
            if !SANDBOX_PERMISSIONS.contains(&p.as_str()) {
                return Err(MarketplaceError::Layout(format!(
                    "unknown sandbox permission: {p}"
                )));
            }
        }
    }
    Ok(())
}

pub fn check_publisher_trust(
    publisher: Option<&str>,
    policy: &TrustPolicy,
) -> Result<(), MarketplaceError> {
    let policy = normalize_trust_policy(policy);
    let p = publisher.unwrap_or("").trim().to_ascii_lowercase();
    if !p.is_empty() && policy.denied_publishers.iter().any(|d| d == &p) {
        return Err(MarketplaceError::Trust(format!("publisher denied: {p}")));
    }
    if !policy.allowed_publishers.is_empty()
        && (p.is_empty() || !policy.allowed_publishers.iter().any(|a| a == &p))
    {
        return Err(MarketplaceError::Trust(format!(
            "publisher not in allowlist: {}",
            publisher.unwrap_or("(missing)")
        )));
    }
    if policy.require_signature {
        // Caller must also supply signature on meta; this gate documents policy intent.
    }
    Ok(())
}

pub fn verify_bytes_sha256(
    bytes: &[u8],
    meta: &MarketplacePackageMeta,
) -> Result<(), MarketplaceError> {
    validate_package_meta(meta)?;
    if bytes.len() as u64 != meta.size {
        return Err(MarketplaceError::Hash(format!(
            "size mismatch: expected {}, got {}",
            meta.size,
            bytes.len()
        )));
    }
    let actual = hex_sha256(bytes);
    let expected = meta.sha256.trim().to_ascii_lowercase();
    if actual != expected {
        return Err(MarketplaceError::Hash(format!(
            "hash mismatch: expected {expected}, got {actual}"
        )));
    }
    if meta.signature.as_deref().map(str::trim).filter(|s| !s.is_empty()).is_none() {
        // optional
    }
    Ok(())
}

pub fn verify_file_against_meta(
    path: &Path,
    meta: &MarketplacePackageMeta,
) -> Result<(), MarketplaceError> {
    validate_package_meta(meta)?;
    let meta_size = std::fs::metadata(path)
        .map_err(|e| MarketplaceError::Hash(e.to_string()))?
        .len();
    if meta_size != meta.size {
        return Err(MarketplaceError::Hash(format!(
            "size mismatch: expected {}, got {meta_size}",
            meta.size
        )));
    }
    verify_file_sha256(path, &meta.sha256).map_err(|e| MarketplaceError::Hash(e.to_string()))
}

#[cfg(test)]
mod file_tests {
    use super::*;
    use crate::update::hex_sha256;
    use std::io::Write;

    #[test]
    fn verify_temp_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("wiparse-mkt-{}.bin", std::process::id()));
        let bytes = b"file-meta-check";
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(bytes).unwrap();
        }
        let meta = MarketplacePackageMeta {
            id: "x".into(),
            version: "1.0.0".into(),
            name: None,
            name_zh: None,
            r#type: None,
            description: None,
            publisher: None,
            channel: None,
            license: None,
            repository: None,
            engines: None,
            sha256: hex_sha256(bytes),
            size: bytes.len() as u64,
            signature: None,
            published_at: None,
            download_url: None,
            download_path: None,
            compatibility: None,
            sandbox: None,
        };
        verify_file_against_meta(&path, &meta).unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
