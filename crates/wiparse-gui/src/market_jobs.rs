//! Background marketplace catalog refresh + install helpers for Testing Hub GUI.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use wiparse_core::marketplace::{
    fetch_catalog, fetch_version_meta, is_loopback_http_url, CatalogQuery, MarketplacePackageMeta,
    PluginCatalogEntry,
};

fn loopback_http_ok(url: &str) -> bool {
    is_loopback_http_url(url)
}

fn enable_loopback_http(url: &str) {
    if loopback_http_ok(url) {
        std::env::set_var("WIPARSE_MARKETPLACE_ALLOW_HTTP", "1");
    } else {
        std::env::remove_var("WIPARSE_MARKETPLACE_ALLOW_HTTP");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HubMode {
    #[default]
    Plugins,
    Market,
}

#[derive(Debug, Clone)]
pub struct CatalogRow {
    pub id: String,
    pub name: String,
    pub name_zh: String,
    pub type_name: String,
    pub publisher: String,
    pub channel: String,
    pub latest_version: String,
    pub description: String,
    pub versions: Vec<String>,
    pub installed: bool,
    pub installed_version: Option<String>,
}

impl CatalogRow {
    pub fn from_entry(e: &PluginCatalogEntry, installed_version: Option<String>) -> Self {
        let latest = e
            .latest_version
            .clone()
            .or_else(|| e.versions.as_ref().and_then(|v| v.first().cloned()))
            .unwrap_or_else(|| "0.0.0".into());
        Self {
            id: e.id.clone(),
            name: e.name.clone().unwrap_or_else(|| e.id.clone()),
            name_zh: e.name_zh.clone().unwrap_or_default(),
            type_name: e.r#type.clone().unwrap_or_else(|| "custom".into()),
            publisher: e.publisher.clone().unwrap_or_default(),
            channel: e.channel.clone().unwrap_or_else(|| "stable".into()),
            latest_version: latest,
            description: e.description.clone().unwrap_or_default(),
            versions: e.versions.clone().unwrap_or_default(),
            installed: installed_version.is_some(),
            installed_version,
        }
    }

    pub fn has_update(&self) -> bool {
        match &self.installed_version {
            Some(v) => !self.latest_version.is_empty() && v != &self.latest_version,
            None => false,
        }
    }
}

#[derive(Debug)]
pub enum MarketJobEvent {
    CatalogOk(Vec<CatalogRow>),
    CatalogErr(String),
    InstallOk {
        id: String,
        version: String,
        dir: String,
    },
    InstallErr {
        id: String,
        message: String,
    },
    UninstallOk {
        id: String,
        version: String,
    },
    UninstallErr {
        id: String,
        message: String,
    },
    Log(String),
}

pub struct MarketJob {
    pub rx: Receiver<MarketJobEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PullResult {
    ok: bool,
    id: Option<String>,
    version: Option<String>,
    dir: Option<String>,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

pub fn spawn_catalog_refresh(
    base_url: String,
    channel: String,
    installed: Vec<(String, String)>,
) -> MarketJob {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        enable_loopback_http(&base_url);
        let q = CatalogQuery {
            channel: if channel.trim().is_empty() {
                None
            } else {
                Some(channel)
            },
            r#type: None,
            q: None,
        };
        match fetch_catalog(&base_url, &q) {
            Ok(resp) => {
                let rows: Vec<CatalogRow> = resp
                    .plugins
                    .iter()
                    .map(|e| {
                        let inst = installed
                            .iter()
                            .find(|(id, _)| id == &e.id)
                            .map(|(_, v)| v.clone());
                        CatalogRow::from_entry(e, inst)
                    })
                    .collect();
                let _ = tx.send(MarketJobEvent::CatalogOk(rows));
            }
            Err(e) => {
                let _ = tx.send(MarketJobEvent::CatalogErr(e.to_string()));
            }
        }
    });
    MarketJob { rx }
}

pub fn spawn_pull_install(
    node: String,
    marketplace_mjs: PathBuf,
    base_url: String,
    plugin_id: String,
    version: String,
    data_root: String,
    install_dir: String,
) -> MarketJob {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(MarketJobEvent::Log(format!(
            "[marketplace] pull {plugin_id}@{version} from {base_url}\n"
        )));
        enable_loopback_http(&base_url);
        if let Err(e) = fetch_version_meta(&base_url, &plugin_id, &version) {
            let _ = tx.send(MarketJobEvent::InstallErr {
                id: plugin_id.clone(),
                message: format!("metadata: {e}"),
            });
            return;
        }
        let mut cmd = Command::new(&node);
        cmd.arg(&marketplace_mjs)
            .args([
                "pull",
                "--plugin",
                &plugin_id,
                "--version",
                &version,
                "--url",
                &base_url,
                "--data-root",
                &data_root,
                "--install-dir",
                &install_dir,
                "--json",
            ])
            .env("WIPARSE_MARKETPLACE_URL", &base_url);
        if loopback_http_ok(&base_url) {
            cmd.env("WIPARSE_MARKETPLACE_ALLOW_HTTP", "1");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        match cmd.output() {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                if !stderr.trim().is_empty() {
                    let _ = tx.send(MarketJobEvent::Log(format!("[marketplace] {stderr}")));
                }
                match serde_json::from_str::<PullResult>(stdout.trim()) {
                    Ok(r) if r.ok => {
                        let _ = tx.send(MarketJobEvent::InstallOk {
                            id: r.id.unwrap_or(plugin_id),
                            version: r.version.unwrap_or(version),
                            dir: r.dir.unwrap_or_default(),
                        });
                    }
                    Ok(r) => {
                        let msg = r
                            .error
                            .as_ref()
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("pull failed")
                            .to_string();
                        let _ = tx.send(MarketJobEvent::InstallErr {
                            id: plugin_id,
                            message: msg,
                        });
                    }
                    Err(_) => {
                        if out.status.success() {
                            let _ = tx.send(MarketJobEvent::InstallOk {
                                id: plugin_id,
                                version,
                                dir: String::new(),
                            });
                        } else {
                            let _ = tx.send(MarketJobEvent::InstallErr {
                                id: plugin_id,
                                message: if stdout.trim().is_empty() {
                                    stderr
                                } else {
                                    stdout
                                },
                            });
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(MarketJobEvent::InstallErr {
                    id: plugin_id,
                    message: e.to_string(),
                });
            }
        }
    });
    MarketJob { rx }
}

pub fn spawn_uninstall(
    node: String,
    marketplace_mjs: PathBuf,
    plugin_id: String,
    version: String,
    data_root: String,
    install_dir: String,
) -> MarketJob {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut cmd = Command::new(&node);
        cmd.arg(&marketplace_mjs).args([
            "uninstall",
            "--plugin",
            &plugin_id,
            "--version",
            &version,
            "--data-root",
            &data_root,
            "--install-dir",
            &install_dir,
            "--json",
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        match cmd.output() {
            Ok(out) if out.status.success() => {
                let _ = tx.send(MarketJobEvent::UninstallOk {
                    id: plugin_id,
                    version,
                });
            }
            Ok(out) => {
                let msg = String::from_utf8_lossy(&out.stderr);
                let msg2 = String::from_utf8_lossy(&out.stdout);
                let _ = tx.send(MarketJobEvent::UninstallErr {
                    id: plugin_id,
                    message: if msg.trim().is_empty() {
                        msg2.to_string()
                    } else {
                        msg.to_string()
                    },
                });
            }
            Err(e) => {
                let _ = tx.send(MarketJobEvent::UninstallErr {
                    id: plugin_id,
                    message: e.to_string(),
                });
            }
        }
    });
    MarketJob { rx }
}

pub fn marketplace_cli_path() -> PathBuf {
    wiparse_core::paths::project_path("test-tools/marketplace.mjs")
}

pub fn installed_map_from_registry(marketplace_root: &Path) -> Vec<(String, String)> {
    let reg = marketplace_root.join("registry.json");
    let Ok(text) = std::fs::read_to_string(reg) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(plugins) = v.get("plugins").and_then(|p| p.as_object()) {
        for (id, entry) in plugins {
            if let Some(active) = entry.get("active").and_then(|a| a.as_str()) {
                out.push((id.clone(), active.to_string()));
            }
        }
    }
    out
}

#[allow(dead_code)]
pub fn fetch_meta(
    base_url: &str,
    id: &str,
    version: &str,
) -> Result<MarketplacePackageMeta, String> {
    fetch_version_meta(base_url, id, version).map_err(|e| e.to_string())
}
