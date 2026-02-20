// boop changelog — history query

use std::path::Path;

use crate::assembly::assemble_changelog_for_workspace;
use crate::errors::{BoopError, ChangelogError};
use crate::store;

pub fn run(
    base: &Path,
    range: Option<&str>,
    workspaces: Option<&str>,
    group: Option<&str>,
) -> Result<(), BoopError> {
    store::ensure_initialized(base).map_err(|_| ChangelogError::NotInitialized)?;
    let manifest = store::read_manifest(base).map_err(ChangelogError::Store)?;
    store::maybe_migrate_to_multi(base, &manifest).map_err(ChangelogError::Store)?;

    // --group mode: print changelog for all workspaces in a release group
    if let Some(group_id) = group {
        let rg = manifest
            .release_groups
            .iter()
            .find(|g| g.id == group_id)
            .ok_or_else(|| ChangelogError::ReleaseGroupNotFound {
                id: group_id.to_string(),
            })?;

        let mut output = String::new();
        for ws_name in &rg.workspaces {
            let ws = manifest.workspaces.get(ws_name).ok_or_else(|| {
                ChangelogError::WorkspaceNotFound {
                    name: ws_name.clone(),
                }
            })?;
            let version =
                rg.after
                    .get(ws_name)
                    .ok_or_else(|| ChangelogError::WorkspaceNotFound {
                        name: ws_name.clone(),
                    })?;
            let release =
                ws.releases
                    .get(version)
                    .ok_or_else(|| ChangelogError::VersionNotFound {
                        version: version.clone(),
                    })?;
            if !output.is_empty() {
                output.push('\n');
            }
            if rg.workspaces.len() > 1 {
                output.push_str(&format!("## {ws_name}\n\n"));
            }
            let section =
                assemble_changelog_for_workspace(base, &manifest, ws_name, &release.entries)
                    .map_err(ChangelogError::Store)?;
            output.push_str(&section);
        }
        print!("{output}");
        return Ok(());
    }

    // Determine target workspace names
    let ws_names: Vec<String> = match workspaces {
        Some(csv) => store::parse_workspace_csv(csv).map_err(ChangelogError::Store)?,
        None => vec![manifest.default_workspace.clone()],
    };

    // Validate all workspace names
    for name in &ws_names {
        if !manifest.workspaces.contains_key(name) {
            return Err(ChangelogError::WorkspaceNotFound { name: name.clone() }.into());
        }
    }

    let multi_display = ws_names.len() > 1;
    let mut full_output = String::new();

    for ws_name in &ws_names {
        let ws = &manifest.workspaces[ws_name];

        if ws.releases.is_empty() {
            if !multi_display {
                println!(
                    "No releases yet. Record changes with `boop major|minor|patch \"message\"`, then run `boop apply` to cut a release."
                );
                return Ok(());
            }
            continue;
        }

        if multi_display && !full_output.is_empty() {
            full_output.push('\n');
        }
        if multi_display {
            full_output.push_str(&format!("## {ws_name}\n\n"));
        }

        let section = match range {
            None => {
                let version = &ws.version;
                let release =
                    ws.releases
                        .get(version)
                        .ok_or_else(|| ChangelogError::VersionNotFound {
                            version: version.clone(),
                        })?;
                assemble_changelog_for_workspace(base, &manifest, ws_name, &release.entries)
                    .map_err(ChangelogError::Store)?
            }
            Some(range_str) if range_str.contains("...") => {
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

                let mut versions_in_range: Vec<semver::Version> = ws
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

                let mut combined = String::new();
                for version in &versions_in_range {
                    let version_str = version.to_string();
                    let release = ws.releases.get(&version_str).ok_or_else(|| {
                        ChangelogError::VersionNotFound {
                            version: version_str.clone(),
                        }
                    })?;
                    let s = assemble_changelog_for_workspace(
                        base,
                        &manifest,
                        ws_name,
                        &release.entries,
                    )
                    .map_err(ChangelogError::Store)?;
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(&s);
                }
                combined
            }
            Some(version_str) => {
                let release = ws.releases.get(version_str).ok_or_else(|| {
                    ChangelogError::VersionNotFound {
                        version: version_str.to_string(),
                    }
                })?;
                assemble_changelog_for_workspace(base, &manifest, ws_name, &release.entries)
                    .map_err(ChangelogError::Store)?
            }
        };

        full_output.push_str(&section);
    }

    print!("{full_output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Manifest, Release, Workspace};
    use std::collections::BTreeMap;
    use std::fs;

    fn setup_test(
        version: &str,
        releases: Vec<(&str, Vec<&str>)>,
        entries: Vec<(&str, &str)>,
    ) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        // Single-workspace: flat changelogs layout
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
                    release_group: None,
                },
            );
        }

        let mut workspaces = BTreeMap::new();
        workspaces.insert(
            "root".to_string(),
            Workspace {
                path: ".".to_string(),
                name: None,
                version: version.to_string(),
                releases: release_map,
            },
        );

        let manifest = Manifest {
            default_workspace: "root".to_string(),
            workspaces,
            groups: BTreeMap::new(),
            release_groups: Vec::new(),
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

        let result = run(dir.path(), None, None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn no_arg_no_releases_prints_guidance() {
        let dir = setup_test("1.3.0", vec![], vec![]);

        let result = run(dir.path(), None, None, None);
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

        let result = run(dir.path(), Some("1.2.0"), None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn single_version_not_found() {
        let dir = setup_test(
            "1.0.0",
            vec![("1.0.0", vec!["patch-01HQ1.md"])],
            vec![("patch-01HQ1.md", "## Fix")],
        );

        let result = run(dir.path(), Some("9.9.9"), None, None);
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

        let result = run(dir.path(), Some("1.0.0...1.1.0"), None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn malformed_range() {
        let dir = setup_test(
            "1.0.0",
            vec![("1.0.0", vec!["patch-01HQ1.md"])],
            vec![("patch-01HQ1.md", "## Fix")],
        );

        let result = run(dir.path(), Some("...1.0.0"), None, None);
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

        let result = run(dir.path(), Some("1.0.0..."), None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid range"));
    }

    #[test]
    fn not_initialized() {
        let dir = tempfile::tempdir().unwrap();

        let result = run(dir.path(), None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
    }

    #[test]
    fn workspace_flag_targets_specific_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let api_changelogs = dir.path().join(".boop/changelogs/api");
        fs::create_dir_all(&api_changelogs).unwrap();
        fs::write(api_changelogs.join("minor-01ABC.md"), "## API feature").unwrap();

        let mut workspaces = BTreeMap::new();
        workspaces.insert(
            "root".to_string(),
            Workspace {
                path: ".".to_string(),
                name: None,
                version: "1.0.0".to_string(),
                releases: BTreeMap::new(),
            },
        );
        let mut api_releases = BTreeMap::new();
        api_releases.insert(
            "2.1.0".to_string(),
            Release {
                entries: vec!["minor-01ABC.md".to_string()],
                release_group: None,
            },
        );
        workspaces.insert(
            "api".to_string(),
            Workspace {
                path: "apps/api".to_string(),
                name: None,
                version: "2.1.0".to_string(),
                releases: api_releases,
            },
        );

        let manifest = Manifest {
            default_workspace: "root".to_string(),
            workspaces,
            groups: BTreeMap::new(),
            release_groups: Vec::new(),
        };
        store::write_manifest(dir.path(), &manifest).unwrap();

        let result = run(dir.path(), None, Some("api"), None);
        assert!(result.is_ok());
    }

    #[test]
    fn multi_workspace_prints_headers() {
        let dir = tempfile::tempdir().unwrap();
        let api_cl = dir.path().join(".boop/changelogs/api");
        let web_cl = dir.path().join(".boop/changelogs/web");
        fs::create_dir_all(&api_cl).unwrap();
        fs::create_dir_all(&web_cl).unwrap();
        fs::write(api_cl.join("minor-01ABC.md"), "## API feature").unwrap();
        fs::write(web_cl.join("patch-01DEF.md"), "## Web fix").unwrap();

        let mut workspaces = BTreeMap::new();

        let mut api_releases = BTreeMap::new();
        api_releases.insert(
            "2.1.0".to_string(),
            Release {
                entries: vec!["minor-01ABC.md".to_string()],
                release_group: None,
            },
        );
        workspaces.insert(
            "api".to_string(),
            Workspace {
                path: "apps/api".to_string(),
                name: None,
                version: "2.1.0".to_string(),
                releases: api_releases,
            },
        );

        let mut web_releases = BTreeMap::new();
        web_releases.insert(
            "0.8.3".to_string(),
            Release {
                entries: vec!["patch-01DEF.md".to_string()],
                release_group: None,
            },
        );
        workspaces.insert(
            "web".to_string(),
            Workspace {
                path: "apps/web".to_string(),
                name: None,
                version: "0.8.3".to_string(),
                releases: web_releases,
            },
        );

        let manifest = Manifest {
            default_workspace: "api".to_string(),
            workspaces,
            groups: BTreeMap::new(),
            release_groups: Vec::new(),
        };
        store::write_manifest(dir.path(), &manifest).unwrap();

        let result = run(dir.path(), None, Some("api,web"), None);
        assert!(result.is_ok());
    }

    #[test]
    fn unknown_workspace_errors() {
        let dir = setup_test(
            "1.0.0",
            vec![("1.0.0", vec!["patch-01HQ1.md"])],
            vec![("patch-01HQ1.md", "## Fix")],
        );

        let result = run(dir.path(), None, Some("nonexistent"), None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("workspace not found"));
    }

    #[test]
    fn group_filter_shows_release_group() {
        use crate::store::ReleaseGroup;

        let dir = tempfile::tempdir().unwrap();
        let api_cl = dir.path().join(".boop/changelogs/api");
        let web_cl = dir.path().join(".boop/changelogs/web");
        fs::create_dir_all(&api_cl).unwrap();
        fs::create_dir_all(&web_cl).unwrap();
        fs::write(api_cl.join("minor-01ABC.md"), "## API feature").unwrap();
        fs::write(web_cl.join("patch-01ABC.md"), "## Web fix").unwrap();

        let mut workspaces = BTreeMap::new();

        let mut api_releases = BTreeMap::new();
        api_releases.insert(
            "2.1.0".to_string(),
            Release {
                entries: vec!["minor-01ABC.md".to_string()],
                release_group: Some("rg-01xyz".to_string()),
            },
        );
        workspaces.insert(
            "api".to_string(),
            Workspace {
                path: "apps/api".to_string(),
                name: None,
                version: "2.1.0".to_string(),
                releases: api_releases,
            },
        );

        let mut web_releases = BTreeMap::new();
        web_releases.insert(
            "0.8.3".to_string(),
            Release {
                entries: vec!["patch-01ABC.md".to_string()],
                release_group: Some("rg-01xyz".to_string()),
            },
        );
        workspaces.insert(
            "web".to_string(),
            Workspace {
                path: "apps/web".to_string(),
                name: None,
                version: "0.8.3".to_string(),
                releases: web_releases,
            },
        );

        let mut before = BTreeMap::new();
        before.insert("api".to_string(), "2.0.4".to_string());
        before.insert("web".to_string(), "0.8.2".to_string());
        let mut after = BTreeMap::new();
        after.insert("api".to_string(), "2.1.0".to_string());
        after.insert("web".to_string(), "0.8.3".to_string());

        let manifest = Manifest {
            default_workspace: "api".to_string(),
            workspaces,
            groups: BTreeMap::new(),
            release_groups: vec![ReleaseGroup {
                id: "rg-01xyz".to_string(),
                workspaces: vec!["api".to_string(), "web".to_string()],
                before,
                after,
            }],
        };
        store::write_manifest(dir.path(), &manifest).unwrap();

        let result = run(dir.path(), None, None, Some("rg-01xyz"));
        assert!(result.is_ok());
    }

    #[test]
    fn unknown_group_errors() {
        let dir = setup_test(
            "1.0.0",
            vec![("1.0.0", vec!["patch-01HQ1.md"])],
            vec![("patch-01HQ1.md", "## Fix")],
        );

        let result = run(dir.path(), None, None, Some("rg-nonexistent"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("release group not found"));
    }
}
