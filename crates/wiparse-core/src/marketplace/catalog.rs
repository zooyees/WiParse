//! Marketplace catalog / package metadata schemas.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const CHANNELS: &[&str] = &["stable", "beta", "internal"];

pub const SANDBOX_PERMISSIONS: &[&str] = &[
    "gui.api",
    "cli",
    "serial",
    "fs.data_root",
    "fs.plugin_dir",
    "network.outbound",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxSpec {
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplacePackageMeta {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub name_zh: Option<String>,
    #[serde(default, rename = "type")]
    pub r#type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub engines: Option<HashMap<String, String>>,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub download_path: Option<String>,
    #[serde(default)]
    pub compatibility: Option<HashMap<String, String>>,
    #[serde(default)]
    pub sandbox: Option<SandboxSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginVersionInfo {
    #[serde(flatten)]
    pub meta: MarketplacePackageMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCatalogEntry {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub name_zh: Option<String>,
    #[serde(default, rename = "type")]
    pub r#type: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub versions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogResponse {
    pub ok: bool,
    #[serde(default)]
    pub plugins: Vec<PluginCatalogEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct CatalogQuery {
    pub channel: Option<String>,
    pub r#type: Option<String>,
    pub q: Option<String>,
}
