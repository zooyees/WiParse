//! HTTPS manifest fetch.

use crate::update::UpdateManifest;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("network: {0}")]
    Network(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Download and parse the remote update manifest (HTTPS only).
pub fn fetch_manifest(url: &str) -> Result<UpdateManifest, FetchError> {
    if !url.starts_with("https://") {
        return Err(FetchError::Network(
            "manifest URL must use HTTPS".into(),
        ));
    }
    let resp = ureq::get(url)
        .call()
        .map_err(|e| FetchError::Network(e.to_string()))?;
    let manifest: UpdateManifest = resp
        .into_json()
        .map_err(|e| FetchError::Network(e.to_string()))?;
    Ok(manifest)
}
