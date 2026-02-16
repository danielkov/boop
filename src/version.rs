// semver resolution logic, pre-release handling

use crate::errors::VersionError;
use semver::{Prerelease, Version};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BumpKind {
    Patch,
    Minor,
    Major,
}

impl std::fmt::Display for BumpKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BumpKind::Major => write!(f, "major"),
            BumpKind::Minor => write!(f, "minor"),
            BumpKind::Patch => write!(f, "patch"),
        }
    }
}

/// Extract bump kind from an entry filename prefix (e.g. "major-01HQ..." → Major).
/// Filename format is `<kind>-<ulid>.md`.
pub fn parse_kind(filename: &str) -> Option<BumpKind> {
    let prefix = filename.split('-').next()?;
    match prefix {
        "major" => Some(BumpKind::Major),
        "minor" => Some(BumpKind::Minor),
        "patch" => Some(BumpKind::Patch),
        _ => None,
    }
}

/// Determine the highest bump kind from a list of entry filenames.
/// Major > Minor > Patch. Returns None if no valid kinds found.
pub fn highest_bump(entries: &[String]) -> Option<BumpKind> {
    entries.iter().filter_map(|f| parse_kind(f)).max()
}

/// Parse a pre-release identifier like "beta.1" into (tag, num).
fn parse_pre(pre: &Prerelease) -> Option<(String, u64)> {
    let s = pre.as_str();
    let dot_pos = s.rfind('.')?;
    let tag = &s[..dot_pos];
    let num = s[dot_pos + 1..].parse::<u64>().ok()?;
    Some((tag.to_string(), num))
}

/// Resolve the next version given current version, bump kind, and pre-release tag.
///
/// `pre_tag` encodes:
/// - `None` → no --pre flag (normal release)
/// - `Some(None)` → `--pre` with no value (use "pre" as tag)
/// - `Some(Some("beta"))` → `--pre beta`
pub fn resolve_next_version(
    current: &Version,
    bump: BumpKind,
    pre_tag: Option<Option<&str>>,
) -> Result<Version, VersionError> {
    let tag = pre_tag.map(|t| t.unwrap_or("pre"));

    if current.pre.is_empty() {
        // Current is a stable release — apply the bump
        let mut next = current.clone();
        match bump {
            BumpKind::Major => {
                next.major += 1;
                next.minor = 0;
                next.patch = 0;
            }
            BumpKind::Minor => {
                next.minor += 1;
                next.patch = 0;
            }
            BumpKind::Patch => {
                next.patch += 1;
            }
        }
        next.pre = Prerelease::EMPTY;

        if let Some(t) = tag {
            next.pre =
                Prerelease::new(&format!("{t}.0")).map_err(|_| VersionError::InvalidVersion {
                    input: format!("{t}.0"),
                })?;
        }

        Ok(next)
    } else {
        // Current is a pre-release
        match tag {
            None => {
                // Strip pre-release, release the core version as stable
                let mut next = current.clone();
                next.pre = Prerelease::EMPTY;
                next.build = semver::BuildMetadata::EMPTY;
                Ok(next)
            }
            Some(t) => {
                let mut next = current.clone();
                if let Some((current_tag, num)) = parse_pre(&current.pre) {
                    if current_tag == t {
                        // Same tag — increment numeric component
                        next.pre = Prerelease::new(&format!("{t}.{}", num + 1)).map_err(|_| {
                            VersionError::InvalidVersion {
                                input: format!("{t}.{}", num + 1),
                            }
                        })?;
                    } else {
                        // Different tag — reset to 0
                        next.pre = Prerelease::new(&format!("{t}.0")).map_err(|_| {
                            VersionError::InvalidVersion {
                                input: format!("{t}.0"),
                            }
                        })?;
                    }
                } else {
                    // Can't parse current pre-release, start fresh
                    next.pre = Prerelease::new(&format!("{t}.0")).map_err(|_| {
                        VersionError::InvalidVersion {
                            input: format!("{t}.0"),
                        }
                    })?;
                }
                next.build = semver::BuildMetadata::EMPTY;
                Ok(next)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    // --- parse_kind tests ---

    #[test]
    fn parse_kind_major() {
        assert_eq!(parse_kind("major-01HQ1XABC.md"), Some(BumpKind::Major));
    }

    #[test]
    fn parse_kind_minor() {
        assert_eq!(parse_kind("minor-01HQ1XABC.md"), Some(BumpKind::Minor));
    }

    #[test]
    fn parse_kind_patch() {
        assert_eq!(parse_kind("patch-01HQ1XABC.md"), Some(BumpKind::Patch));
    }

    #[test]
    fn parse_kind_unknown() {
        assert_eq!(parse_kind("fix-01HQ1XABC.md"), None);
    }

    #[test]
    fn parse_kind_empty() {
        assert_eq!(parse_kind(""), None);
    }

    // --- highest_bump tests ---

    #[test]
    fn highest_bump_mixed() {
        let entries = vec![
            "patch-01HQ1.md".to_string(),
            "minor-01HQ2.md".to_string(),
            "patch-01HQ3.md".to_string(),
        ];
        assert_eq!(highest_bump(&entries), Some(BumpKind::Minor));
    }

    #[test]
    fn highest_bump_major_wins() {
        let entries = vec![
            "minor-01HQ1.md".to_string(),
            "major-01HQ2.md".to_string(),
            "patch-01HQ3.md".to_string(),
        ];
        assert_eq!(highest_bump(&entries), Some(BumpKind::Major));
    }

    #[test]
    fn highest_bump_only_patches() {
        let entries = vec!["patch-01HQ1.md".to_string(), "patch-01HQ2.md".to_string()];
        assert_eq!(highest_bump(&entries), Some(BumpKind::Patch));
    }

    #[test]
    fn highest_bump_empty() {
        let entries: Vec<String> = vec![];
        assert_eq!(highest_bump(&entries), None);
    }

    #[test]
    fn highest_bump_invalid_entries() {
        let entries = vec!["foo-01HQ1.md".to_string(), "bar-01HQ2.md".to_string()];
        assert_eq!(highest_bump(&entries), None);
    }

    // --- RFC table cases for resolve_next_version ---

    #[test]
    fn rfc_case_1_stable_minor_no_pre() {
        // 1.2.3 + minor + no --pre → 1.3.0
        let current = Version::parse("1.2.3").unwrap();
        let result = resolve_next_version(&current, BumpKind::Minor, None).unwrap();
        assert_eq!(result, Version::parse("1.3.0").unwrap());
    }

    #[test]
    fn rfc_case_2_stable_minor_pre_default() {
        // 1.2.3 + minor + --pre → 1.3.0-pre.0
        let current = Version::parse("1.2.3").unwrap();
        let result = resolve_next_version(&current, BumpKind::Minor, Some(None)).unwrap();
        assert_eq!(result, Version::parse("1.3.0-pre.0").unwrap());
    }

    #[test]
    fn rfc_case_3_stable_minor_pre_beta() {
        // 1.2.3 + minor + --pre beta → 1.3.0-beta.0
        let current = Version::parse("1.2.3").unwrap();
        let result = resolve_next_version(&current, BumpKind::Minor, Some(Some("beta"))).unwrap();
        assert_eq!(result, Version::parse("1.3.0-beta.0").unwrap());
    }

    #[test]
    fn rfc_case_4_pre_same_tag_increment() {
        // 1.3.0-pre.0 + any + --pre → 1.3.0-pre.1
        let current = Version::parse("1.3.0-pre.0").unwrap();
        let result = resolve_next_version(&current, BumpKind::Patch, Some(None)).unwrap();
        assert_eq!(result, Version::parse("1.3.0-pre.1").unwrap());
    }

    #[test]
    fn rfc_case_5_pre_same_named_tag_increment() {
        // 1.3.0-beta.0 + any + --pre beta → 1.3.0-beta.1
        let current = Version::parse("1.3.0-beta.0").unwrap();
        let result = resolve_next_version(&current, BumpKind::Patch, Some(Some("beta"))).unwrap();
        assert_eq!(result, Version::parse("1.3.0-beta.1").unwrap());
    }

    #[test]
    fn rfc_case_6_pre_release_to_stable() {
        // 1.3.0-pre.0 + minor + no --pre → 1.3.0
        let current = Version::parse("1.3.0-pre.0").unwrap();
        let result = resolve_next_version(&current, BumpKind::Minor, None).unwrap();
        assert_eq!(result, Version::parse("1.3.0").unwrap());
    }

    #[test]
    fn rfc_case_7_pre_different_tag() {
        // 1.3.0-beta.0 + any + --pre rc → 1.3.0-rc.0
        let current = Version::parse("1.3.0-beta.0").unwrap();
        let result = resolve_next_version(&current, BumpKind::Patch, Some(Some("rc"))).unwrap();
        assert_eq!(result, Version::parse("1.3.0-rc.0").unwrap());
    }

    // --- Additional edge cases ---

    #[test]
    fn zero_version_follows_same_rules() {
        // 0.1.0 + minor → 0.2.0 (no special 0.x handling)
        let current = Version::parse("0.1.0").unwrap();
        let result = resolve_next_version(&current, BumpKind::Minor, None).unwrap();
        assert_eq!(result, Version::parse("0.2.0").unwrap());
    }

    #[test]
    fn zero_version_major_bump() {
        let current = Version::parse("0.1.2").unwrap();
        let result = resolve_next_version(&current, BumpKind::Major, None).unwrap();
        assert_eq!(result, Version::parse("1.0.0").unwrap());
    }

    #[test]
    fn zero_version_patch_bump() {
        let current = Version::parse("0.0.1").unwrap();
        let result = resolve_next_version(&current, BumpKind::Patch, None).unwrap();
        assert_eq!(result, Version::parse("0.0.2").unwrap());
    }

    #[test]
    fn pre_release_higher_numeric() {
        // 2.0.0-alpha.9 + --pre alpha → 2.0.0-alpha.10
        let current = Version::parse("2.0.0-alpha.9").unwrap();
        let result = resolve_next_version(&current, BumpKind::Patch, Some(Some("alpha"))).unwrap();
        assert_eq!(result, Version::parse("2.0.0-alpha.10").unwrap());
    }
}
