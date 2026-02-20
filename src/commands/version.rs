use std::path::Path;

use crate::errors::VersionError;
use crate::store;

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
            .ok_or(VersionError::Store(crate::errors::StoreError::NoDefaultWorkspace))?,
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
}
