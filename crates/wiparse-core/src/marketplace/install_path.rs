//! Least-privilege install path helpers for marketplace plugins.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct MarketplacePaths {
    pub root: PathBuf,
}

impl MarketplacePaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn plugins(&self) -> PathBuf {
        self.root.join("plugins")
    }

    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn registry(&self) -> PathBuf {
        registry_path(&self.root)
    }

    pub fn plugin_version(&self, id: &str, version: &str) -> Result<PathBuf, String> {
        plugin_version_dir(&self.root, id, version)
    }
}

pub fn default_marketplace_root(data_root: &Path) -> PathBuf {
    data_root.join("marketplace")
}

pub fn registry_path(marketplace_root: &Path) -> PathBuf {
    marketplace_root.join("registry.json")
}

pub fn plugin_version_dir(
    marketplace_root: &Path,
    id: &str,
    version: &str,
) -> Result<PathBuf, String> {
    assert_safe_id(id)?;
    assert_safe_version(version)?;
    Ok(marketplace_root.join("plugins").join(id).join(version))
}

fn assert_safe_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || !id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(format!("invalid plugin id: {id}"));
    }
    Ok(())
}

fn assert_safe_version(version: &str) -> Result<(), String> {
    let v = version.trim();
    if v.is_empty() || v.contains("..") || v.contains('/') || v.contains('\\') || v.contains('\0')
    {
        return Err(format!("invalid version: {version}"));
    }
    if !v
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        || !v
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
    {
        return Err(format!("invalid version chars: {version}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_escape_version() {
        assert!(plugin_version_dir(Path::new("/tmp/m"), "ok", "../x").is_err());
        assert!(plugin_version_dir(Path::new("/tmp/m"), "../x", "1.0.0").is_err());
    }
}
