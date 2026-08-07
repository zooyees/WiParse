//! Update manifest schema served by the release server.

use serde::{Deserialize, Serialize};

/// Platform artifact selector (matches manifest `target` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateTarget {
    WindowsX64,
}

impl UpdateTarget {
    pub fn current() -> Self {
        Self::WindowsX64
    }
}

/// One downloadable release artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePackage {
    pub target: UpdateTarget,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    /// Optional detached signature (base64). Verified when `UPDATE_PUBLIC_KEY` is set.
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
}

/// Remote update manifest (`latest.json` on HTTPS).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub product: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    pub version: String,
    #[serde(default)]
    pub min_version: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub notes_url: Option<String>,
    pub packages: Vec<UpdatePackage>,
}

fn default_channel() -> String {
    "stable".into()
}

impl UpdateManifest {
    pub fn package_for(&self, target: UpdateTarget) -> Option<&UpdatePackage> {
        self.packages.iter().find(|p| p.target == target)
    }
}
