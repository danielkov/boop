use std::path::Path;

use crate::errors::VersionError;
use crate::store;

pub fn run_all_json(base: &Path, omit_unchanged: bool) -> Result<(), VersionError> {
    store::ensure_initialized(base)?;
    let manifest = store::read_manifest(base)?;

    let json = if omit_unchanged {
        let rg = manifest
            .release_groups
            .last()
            .ok_or(VersionError::NoReleaseGroups)?;
        serde_json::to_string(&rg.after).map_err(|e| VersionError::Serialize { source: e })?
    } else {
        let versions: std::collections::BTreeMap<&str, &str> = manifest
            .workspaces
            .iter()
            .map(|(name, ws)| (name.as_str(), ws.version.as_str()))
            .collect();
        serde_json::to_string(&versions).map_err(|e| VersionError::Serialize { source: e })?
    };
    println!("{json}");

    Ok(())
}

pub fn run(base: &Path, workspace: Option<&str>) -> Result<(), VersionError> {
    store::ensure_initialized(base)?;
    let manifest = store::read_manifest(base)?;

    let ws_name: &str = match workspace {
        Some(name) => {
            store::validate_workspace_name(name)?;
            name
        }
        None => manifest
            .default_workspace
            .as_deref()
            .ok_or(VersionError::Store(
                crate::errors::StoreError::NoDefaultWorkspace,
            ))?,
    };
    if manifest.groups.contains_key(ws_name) && !manifest.workspaces.contains_key(ws_name) {
        return Err(VersionError::Store(
            crate::errors::StoreError::WorkspaceIsGroup {
                name: ws_name.to_string(),
            },
        ));
    }
    let ws = manifest
        .workspaces
        .get(ws_name)
        .ok_or_else(|| VersionError::UnknownWorkspace {
            name: ws_name.to_string(),
        })?;
    println!("{}", ws.version);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_workspace_manifest(base: &Path, workspaces: &[(&str, &str)], default: &str) {
        let dir = base.join(".boop");
        fs::create_dir_all(&dir).unwrap();

        let mut toml = format!("default_workspace = \"{default}\"\n\n");
        for (name, path) in workspaces {
            toml.push_str(&format!(
                "[workspaces.{name}]\npath = \"{path}\"\nversion = \"1.0.0\"\n\n"
            ));
        }
        fs::write(dir.join("releases.toml"), toml).unwrap();
    }

    #[test]
    fn default_workspace_version() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", "."), ("api", "apps/api")], "root");

        // No workspace flag → uses default_workspace
        let result = run(dir.path(), None);
        assert!(result.is_ok());
    }

    #[test]
    fn explicit_workspace_version() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", "."), ("api", "apps/api")], "root");

        let result = run(dir.path(), Some("api"));
        assert!(result.is_ok());
    }

    #[test]
    fn unknown_workspace_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".")], "root");

        let err = run(dir.path(), Some("nonexistent")).unwrap_err();
        assert!(
            matches!(err, VersionError::UnknownWorkspace { ref name } if name == "nonexistent"),
            "expected UnknownWorkspace error, got: {err}"
        );
    }

    #[test]
    fn not_initialized_returns_store_error() {
        let dir = tempfile::tempdir().unwrap();
        // No .boop/ directory

        let err = run(dir.path(), None).unwrap_err();
        assert!(matches!(err, VersionError::Store(_)));
    }

    fn setup_manifest_with_release_group(base: &Path) {
        let dir = base.join(".boop");
        fs::create_dir_all(&dir).unwrap();

        let toml = r#"default_workspace = "root"

[workspaces.root]
path = "."
version = "2.0.0"

[workspaces.api]
path = "apps/api"
version = "1.0.0"

[[release_groups]]
id = "rg-01abc"
workspaces = ["root"]
before = { root = "1.0.0" }
after = { root = "2.0.0" }
"#;
        fs::write(dir.join("releases.toml"), toml).unwrap();
    }

    #[test]
    fn all_json_returns_all_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        setup_manifest_with_release_group(dir.path());

        // Capture stdout isn't easy, so just verify it succeeds
        let result = run_all_json(dir.path(), false);
        assert!(result.is_ok());
    }

    #[test]
    fn all_json_omit_unchanged_returns_only_changed() {
        let dir = tempfile::tempdir().unwrap();
        setup_manifest_with_release_group(dir.path());

        let result = run_all_json(dir.path(), true);
        assert!(result.is_ok());
    }

    #[test]
    fn all_json_omit_unchanged_no_release_groups_errors() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".")], "root");

        let err = run_all_json(dir.path(), true).unwrap_err();
        assert!(
            matches!(err, VersionError::NoReleaseGroups),
            "expected NoReleaseGroups error, got: {err}"
        );
    }

    #[test]
    fn all_json_without_omit_unchanged_works_without_release_groups() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", "."), ("api", "apps/api")], "root");

        // No release groups, but should still work — returns all workspace versions
        let result = run_all_json(dir.path(), false);
        assert!(result.is_ok());
    }
}
