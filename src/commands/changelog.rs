// boop changelog — history query

use std::path::Path;

use crate::assembly::assemble_changelog;
use crate::errors::{BoopError, ChangelogError};
use crate::store;

pub fn run(base: &Path, range: Option<&str>) -> Result<(), BoopError> {
    store::ensure_initialized(base).map_err(|_| ChangelogError::NotInitialized)?;
    let manifest = store::read_manifest(base).map_err(ChangelogError::Store)?;

    if manifest.releases.is_empty() {
        println!("No releases yet. Record changes with `boop major|minor|patch \"message\"`, then run `boop apply` to cut a release.");
        return Ok(());
    }

    match range {
        None => {
            // Print changelog for current version
            let version = &manifest.version;
            let release =
                manifest
                    .releases
                    .get(version)
                    .ok_or_else(|| ChangelogError::VersionNotFound {
                        version: version.clone(),
                    })?;
            let output = assemble_changelog(base, version, &release.entries)
                .map_err(ChangelogError::Store)?;
            print!("{output}");
        }
        Some(range_str) if range_str.contains("...") => {
            // Range query
            let parts: Vec<&str> = range_str.splitn(2, "...").collect();
            if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Err(ChangelogError::InvalidRange {
                    input: range_str.to_string(),
                }
                .into());
            }

            let start =
                semver::Version::parse(parts[0]).map_err(|_| ChangelogError::InvalidRange {
                    input: range_str.to_string(),
                })?;
            let end =
                semver::Version::parse(parts[1]).map_err(|_| ChangelogError::InvalidRange {
                    input: range_str.to_string(),
                })?;

            // Collect versions in range, sorted ascending
            let mut versions_in_range: Vec<semver::Version> = manifest
                .releases
                .keys()
                .filter_map(|v| semver::Version::parse(v).ok())
                .filter(|v| v >= &start && v <= &end)
                .collect();
            versions_in_range.sort();

            if versions_in_range.is_empty() {
                return Err(ChangelogError::VersionNotFound {
                    version: range_str.to_string(),
                }
                .into());
            }

            let mut output = String::new();
            for version in &versions_in_range {
                let version_str = version.to_string();
                let release = manifest.releases.get(&version_str).ok_or_else(|| {
                    ChangelogError::VersionNotFound {
                        version: version_str.clone(),
                    }
                })?;
                let section = assemble_changelog(base, &version_str, &release.entries)
                    .map_err(ChangelogError::Store)?;
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(&section);
            }
            print!("{output}");
        }
        Some(version_str) => {
            // Single version query
            let release = manifest.releases.get(version_str).ok_or_else(|| {
                ChangelogError::VersionNotFound {
                    version: version_str.to_string(),
                }
            })?;
            let output = assemble_changelog(base, version_str, &release.entries)
                .map_err(ChangelogError::Store)?;
            print!("{output}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Manifest, Release};
    use std::collections::BTreeMap;
    use std::fs;

    fn setup_test(
        version: &str,
        releases: Vec<(&str, Vec<&str>)>,
        entries: Vec<(&str, &str)>,
    ) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let changelogs = dir.path().join(".boop/changelogs");
        fs::create_dir_all(&changelogs).unwrap();

        for (filename, content) in entries {
            fs::write(changelogs.join(filename), content).unwrap();
        }

        let mut release_map = BTreeMap::new();
        for (ver, entry_names) in releases {
            release_map.insert(
                ver.to_string(),
                Release {
                    entries: entry_names.iter().map(|s| s.to_string()).collect(),
                },
            );
        }

        let manifest = Manifest {
            version: version.to_string(),
            releases: release_map,
        };
        store::write_manifest(dir.path(), &manifest).unwrap();

        dir
    }

    #[test]
    fn no_arg_prints_current_version() {
        let dir = setup_test(
            "1.2.0",
            vec![("1.2.0", vec!["minor-01HQ1.md"])],
            vec![("minor-01HQ1.md", "## Added feature")],
        );

        let result = run(dir.path(), None);
        assert!(result.is_ok());
    }

    #[test]
    fn no_arg_no_releases_prints_guidance() {
        let dir = setup_test("1.3.0", vec![], vec![]);

        let result = run(dir.path(), None);
        assert!(result.is_ok());
    }

    #[test]
    fn single_version_lookup() {
        let dir = setup_test(
            "1.3.0",
            vec![
                ("1.2.0", vec!["minor-01HQ1.md"]),
                ("1.3.0", vec!["patch-01HQ2.md"]),
            ],
            vec![
                ("minor-01HQ1.md", "## Feature"),
                ("patch-01HQ2.md", "## Fix"),
            ],
        );

        let result = run(dir.path(), Some("1.2.0"));
        assert!(result.is_ok());
    }

    #[test]
    fn single_version_not_found() {
        let dir = setup_test(
            "1.0.0",
            vec![("1.0.0", vec!["patch-01HQ1.md"])],
            vec![("patch-01HQ1.md", "## Fix")],
        );

        let result = run(dir.path(), Some("9.9.9"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("9.9.9"));
    }

    #[test]
    fn range_query() {
        let dir = setup_test(
            "2.0.0",
            vec![
                ("1.0.0", vec!["patch-01HQ1.md"]),
                ("1.1.0", vec!["minor-01HQ2.md"]),
                ("2.0.0", vec!["major-01HQ3.md"]),
            ],
            vec![
                ("patch-01HQ1.md", "## Fix v1.0"),
                ("minor-01HQ2.md", "## Feature v1.1"),
                ("major-01HQ3.md", "## Breaking v2.0"),
            ],
        );

        let result = run(dir.path(), Some("1.0.0...1.1.0"));
        assert!(result.is_ok());
    }

    #[test]
    fn malformed_range() {
        let dir = setup_test(
            "1.0.0",
            vec![("1.0.0", vec!["patch-01HQ1.md"])],
            vec![("patch-01HQ1.md", "## Fix")],
        );

        let result = run(dir.path(), Some("...1.0.0"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid range"));
    }

    #[test]
    fn malformed_range_no_end() {
        let dir = setup_test(
            "1.0.0",
            vec![("1.0.0", vec!["patch-01HQ1.md"])],
            vec![("patch-01HQ1.md", "## Fix")],
        );

        let result = run(dir.path(), Some("1.0.0..."));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid range"));
    }

    #[test]
    fn not_initialized() {
        let dir = tempfile::tempdir().unwrap();

        let result = run(dir.path(), None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
    }
}
