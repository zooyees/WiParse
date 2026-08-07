//! Semantic version parsing and comparison (major.minor.patch).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionCmp {
    Less,
    Equal,
    Greater,
}

/// Parse `major.minor.patch` with optional `-prerelease` suffix (prerelease ignored for ordering).
pub fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let core = s.split(['-', '+']).next()?.trim();
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

pub fn compare_versions(a: &str, b: &str) -> Option<VersionCmp> {
    let (a0, a1, a2) = parse_semver(a)?;
    let (b0, b1, b2) = parse_semver(b)?;
    Some(if (a0, a1, a2) < (b0, b1, b2) {
        VersionCmp::Less
    } else if (a0, a1, a2) > (b0, b1, b2) {
        VersionCmp::Greater
    } else {
        VersionCmp::Equal
    })
}

/// True when `remote` is strictly newer than `local`.
pub fn is_newer_version(remote: &str, local: &str) -> bool {
    matches!(compare_versions(remote, local), Some(VersionCmp::Greater))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_order() {
        assert!(is_newer_version("1.0.2", "1.0.1"));
        assert!(!is_newer_version("1.0.1", "1.0.2"));
        assert!(!is_newer_version("1.0.1", "1.0.1"));
        assert!(is_newer_version("1.1.0", "1.0.9"));
    }
}
