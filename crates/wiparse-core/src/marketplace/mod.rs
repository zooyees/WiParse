//! Testing Hub plugin marketplace: catalog types, trust, install paths, HTTPS fetch.
//!
//! Mirrors `test-tools/lib/marketplace-*.mjs`. HTTP download orchestration for
//! install may live in GUI/CLI; this module owns contracts and verification.

mod catalog;
mod install_path;
mod trust;

pub use catalog::{
    CatalogQuery, CatalogResponse, MarketplacePackageMeta, PluginCatalogEntry, PluginVersionInfo,
    SandboxSpec, CHANNELS, SANDBOX_PERMISSIONS,
};
pub use install_path::{
    default_marketplace_root, plugin_version_dir, registry_path, MarketplacePaths,
};
pub use trust::{
    check_publisher_trust, normalize_trust_policy, validate_package_meta, verify_bytes_sha256,
    MarketplaceError, TrustPolicy,
};

use crate::config::MarketplaceConfig;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("network: {0}")]
    Network(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Marketplace(#[from] MarketplaceError),
}

/// Resolve effective marketplace base URL (config + env). Empty → disabled.
pub fn effective_base_url(cfg: &MarketplaceConfig) -> Option<String> {
    if !cfg.enabled {
        return None;
    }
    if let Ok(env) = std::env::var("WIPARSE_MARKETPLACE_URL") {
        let t = env.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    let url = cfg.base_url.trim();
    if url.is_empty() {
        return None;
    }
    Some(url.to_string())
}

pub fn allow_http() -> bool {
    matches!(
        std::env::var("WIPARSE_MARKETPLACE_ALLOW_HTTP")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// True for `http://127.0.0.1`, `http://localhost`, and `http://[::1]` (any port/path).
pub fn is_loopback_http_url(url: &str) -> bool {
    let rest = url
        .trim()
        .strip_prefix("http://")
        .or_else(|| url.trim().strip_prefix("HTTP://"));
    let Some(rest) = rest else {
        return false;
    };
    let hostport = rest.split('/').next().unwrap_or(rest);
    let host = if let Some(inner) = hostport.strip_prefix('[') {
        inner.split(']').next().unwrap_or("")
    } else if let Some((h, port)) = hostport.rsplit_once(':') {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            hostport
        }
    } else {
        hostport
    };
    matches!(
        host.trim().to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

fn assert_url_scheme(url: &str) -> Result<(), MarketplaceError> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if url.starts_with("http://") && (allow_http() || is_loopback_http_url(url)) {
        return Ok(());
    }
    Err(MarketplaceError::Https(format!(
        "URL must use HTTPS: {url}"
    )))
}

/// Fetch remote catalog JSON.
pub fn fetch_catalog(base_url: &str, query: &CatalogQuery) -> Result<CatalogResponse, FetchError> {
    assert_url_scheme(base_url)?;
    let base = base_url.trim_end_matches('/');
    let mut url = format!("{base}/v1/catalog");
    let mut qs = Vec::new();
    if let Some(c) = &query.channel {
        qs.push(format!("channel={}", urlencoding_minimal(c)));
    }
    if let Some(t) = &query.r#type {
        qs.push(format!("type={}", urlencoding_minimal(t)));
    }
    if let Some(q) = &query.q {
        qs.push(format!("q={}", urlencoding_minimal(q)));
    }
    if !qs.is_empty() {
        url.push('?');
        url.push_str(&qs.join("&"));
    }
    let resp = ureq::get(&url)
        .call()
        .map_err(|e| FetchError::Network(e.to_string()))?;
    let body: CatalogResponse = resp
        .into_json()
        .map_err(|e| FetchError::Network(e.to_string()))?;
    Ok(body)
}

/// Fetch single version metadata.
pub fn fetch_version_meta(
    base_url: &str,
    id: &str,
    version: &str,
) -> Result<MarketplacePackageMeta, FetchError> {
    assert_url_scheme(base_url)?;
    let base = base_url.trim_end_matches('/');
    let url = format!(
        "{base}/v1/plugins/{}/versions/{}",
        urlencoding_minimal(id),
        urlencoding_minimal(version)
    );
    let resp = ureq::get(&url)
        .call()
        .map_err(|e| FetchError::Network(e.to_string()))?;
    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| FetchError::Network(e.to_string()))?;
    let meta = if v.get("version").is_some() && v.get("sha256").is_none() {
        serde_json::from_value(v.get("version").cloned().unwrap_or(v))?
    } else {
        serde_json::from_value(v)?
    };
    Ok(meta)
}

fn urlencoding_minimal(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::hex_sha256;

    #[test]
    fn meta_validation_and_hash() {
        let bytes = b"hello-marketplace";
        let sha = hex_sha256(bytes);
        let meta = MarketplacePackageMeta {
            id: "example-smoke".into(),
            version: "1.0.0".into(),
            name: Some("Example".into()),
            name_zh: None,
            r#type: Some("smoke".into()),
            description: None,
            publisher: Some("wiparse".into()),
            channel: Some("stable".into()),
            license: None,
            repository: None,
            engines: None,
            sha256: sha.clone(),
            size: bytes.len() as u64,
            signature: None,
            published_at: None,
            download_url: None,
            download_path: None,
            compatibility: None,
            sandbox: None,
        };
        validate_package_meta(&meta).unwrap();
        verify_bytes_sha256(bytes, &meta).unwrap();
        let policy = TrustPolicy {
            require_signature: false,
            public_keys: vec![],
            allowed_publishers: vec!["wiparse".into()],
            denied_publishers: vec![],
        };
        check_publisher_trust(meta.publisher.as_deref(), &policy).unwrap();
    }

    #[test]
    fn deny_publisher() {
        let policy = TrustPolicy {
            require_signature: false,
            public_keys: vec![],
            allowed_publishers: vec![],
            denied_publishers: vec!["evil".into()],
        };
        let err = check_publisher_trust(Some("evil"), &policy).unwrap_err();
        assert!(matches!(err, MarketplaceError::Trust(_)));
    }

    #[test]
    fn loopback_http_is_allowed_without_env() {
        assert!(is_loopback_http_url("http://127.0.0.1:8787"));
        assert!(is_loopback_http_url("http://localhost/v1/catalog"));
        assert!(is_loopback_http_url("http://[::1]:8787/v1/health"));
        assert!(!is_loopback_http_url("https://127.0.0.1:8787"));
        assert!(!is_loopback_http_url("http://example.com"));
        assert!(assert_url_scheme("http://127.0.0.1:8787").is_ok());
        assert!(assert_url_scheme("http://evil.example").is_err());
    }
}
