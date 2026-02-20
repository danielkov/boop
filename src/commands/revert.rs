// boop revert — roll back the most recent apply group

use std::path::Path;

use crate::errors::RevertError;
use crate::store;

pub fn run(base: &Path) -> Result<(), RevertError> {
    store::ensure_initialized(base)?;
    let mut manifest = store::read_manifest(base)?;

    if let Some(group) = manifest.release_groups.pop() {
        for ws_name in &group.workspaces {
            if let Some(ws) = manifest.workspaces.get_mut(ws_name) {
                // Remove the release version from releases
                if let Some(after_version) = group.after.get(ws_name) {
                    ws.releases.remove(after_version);
                }
                // Reset version to before
                if let Some(before_version) = group.before.get(ws_name) {
                    ws.version = before_version.clone();
                }
            }
        }

        store::write_manifest(base, &manifest)?;

        // Print what was reverted
        for ws_name in &group.workspaces {
            if let (Some(before), Some(after)) =
                (group.before.get(ws_name), group.after.get(ws_name))
            {
                eprintln!("Reverted {ws_name}: {after} → {before}");
            }
        }

        return Ok(());
    }

    // Single-workspace fallback when no release_groups exist:
    // remove current release and roll back to the previous semver key.
    if manifest.workspaces.len() != 1 {
        return Err(RevertError::NothingToRevert);
    }

    let ws_name = manifest.default_workspace.clone();
    let (current_version, next_version) = {
        let ws = manifest
            .workspaces
            .get_mut(&ws_name)
            .ok_or(RevertError::NothingToRevert)?;

        let current_version = ws.version.clone();
        let removed = ws
            .releases
            .remove(&current_version)
            .ok_or(RevertError::NothingToRevert)?;

        let current_semver =
            semver::Version::parse(&current_version).map_err(|_| RevertError::NoPriorVersion)?;

        let mut previous: Option<semver::Version> = None;
        for version in ws.releases.keys() {
            if let Ok(v) = semver::Version::parse(version)
                && v < current_semver
                && previous.as_ref().is_none_or(|p| v > *p)
            {
                previous = Some(v);
            }
        }

        if let Some(prev) = previous {
            ws.version = prev.to_string();
            (current_version, Some(ws.version.clone()))
        } else {
            ws.releases.insert(current_version.clone(), removed);
            (current_version, None)
        }
    };

    if let Some(prev) = next_version {
        store::write_manifest(base, &manifest)?;
        eprintln!("Reverted {ws_name}: {current_version} → {prev}");
        Ok(())
    } else {
        Err(RevertError::NoPriorVersion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_workspace_manifest(base: &Path, workspaces: &[(&str, &str, &str)], default: &str) {
        let dir = base.join(".boop");
        fs::create_dir_all(&dir).unwrap();

        let mut toml = format!("default_workspace = \"{default}\"\n\n");
        for (name, path, version) in workspaces {
            toml.push_str(&format!(
                "[workspaces.{name}]\npath = \"{path}\"\nversion = \"{version}\"\n\n"
            ));
        }
        fs::write(dir.join("releases.toml"), toml).unwrap();
    }

    fn add_entry(base: &Path, workspace: &str, filename: &str, content: &str) {
        let manifest = store::read_manifest(base).unwrap();
        let multi = store::is_multi_workspace(&manifest);
        store::write_entry_scoped(base, workspace, filename, content, multi).unwrap();
    }

    #[test]
    fn apply_then_revert_restores_original_state() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        add_entry(dir.path(), "root", "minor-01abc.md", "## Feature");

        // Apply
        crate::commands::apply::run(dir.path(), None, false, None, false, false).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.1.0");

        // Revert
        run(dir.path()).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.0.0");
        assert!(!root.releases.contains_key("1.1.0"));
        assert!(manifest.release_groups.is_empty());
    }

    #[test]
    fn revert_multi_workspace_group_reverts_all() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(
            dir.path(),
            &[
                ("root", ".", "1.0.0"),
                ("api", "apps/api", "2.0.0"),
                ("web", "apps/web", "0.5.0"),
            ],
            "root",
        );
        add_entry(dir.path(), "api", "minor-01abc.md", "## API feature");
        add_entry(dir.path(), "web", "patch-01def.md", "## Web fix");

        // Apply to api and web together
        crate::commands::apply::run(dir.path(), None, false, Some("api,web"), false, false)
            .unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get("api").unwrap().version, "2.1.0");
        assert_eq!(manifest.workspaces.get("web").unwrap().version, "0.5.1");
        assert_eq!(manifest.release_groups.len(), 1);

        // Revert
        run(dir.path()).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get("api").unwrap().version, "2.0.0");
        assert_eq!(manifest.workspaces.get("web").unwrap().version, "0.5.0");
        assert!(
            !manifest
                .workspaces
                .get("api")
                .unwrap()
                .releases
                .contains_key("2.1.0")
        );
        assert!(
            !manifest
                .workspaces
                .get("web")
                .unwrap()
                .releases
                .contains_key("0.5.1")
        );
        assert!(manifest.release_groups.is_empty());

        // root untouched
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.0.0");
    }

    #[test]
    fn reverted_entries_become_pending_again() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        add_entry(dir.path(), "root", "minor-01abc.md", "## Feature");

        // Apply
        crate::commands::apply::run(dir.path(), None, false, None, false, false).unwrap();

        // Verify entry is no longer pending
        let manifest = store::read_manifest(dir.path()).unwrap();
        let all_entries = store::list_entry_filenames(dir.path(), "root").unwrap();
        let pending = store::pending_entries(&manifest, "root", &all_entries);
        assert!(pending.is_empty());

        // Revert
        run(dir.path()).unwrap();

        // Entry should be pending again (changelog file still on disk)
        let manifest = store::read_manifest(dir.path()).unwrap();
        let all_entries = store::list_entry_filenames(dir.path(), "root").unwrap();
        let pending = store::pending_entries(&manifest, "root", &all_entries);
        assert_eq!(pending, vec!["minor-01abc.md"]);
    }

    #[test]
    fn revert_nothing_to_revert_error() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");

        let err = run(dir.path()).unwrap_err();
        assert!(matches!(err, RevertError::NothingToRevert));
    }

    #[test]
    fn revert_not_initialized_error() {
        let dir = tempfile::tempdir().unwrap();

        let err = run(dir.path()).unwrap_err();
        assert!(matches!(
            err,
            RevertError::Store(crate::errors::StoreError::NotInitialized)
        ));
    }

    #[test]
    fn revert_only_pops_last_group() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(
            dir.path(),
            &[("root", ".", "1.0.0"), ("api", "apps/api", "2.0.0")],
            "root",
        );

        // First apply: root
        add_entry(dir.path(), "root", "minor-01abc.md", "## Root feature");
        crate::commands::apply::run(dir.path(), None, false, Some("root"), false, false).unwrap();

        // Second apply: api
        add_entry(dir.path(), "api", "patch-01def.md", "## API fix");
        crate::commands::apply::run(dir.path(), None, false, Some("api"), false, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.release_groups.len(), 2);
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.1.0");
        assert_eq!(manifest.workspaces.get("api").unwrap().version, "2.0.1");

        // Revert should only undo the second apply (api)
        run(dir.path()).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.release_groups.len(), 1);
        assert_eq!(manifest.workspaces.get("api").unwrap().version, "2.0.0");
        // root should still be at 1.1.0
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.1.0");
    }

    #[test]
    fn revert_single_without_release_groups_uses_previous_semver() {
        let dir = tempfile::tempdir().unwrap();
        let boop_dir = dir.path().join(".boop");
        fs::create_dir_all(&boop_dir).unwrap();
        fs::write(
            boop_dir.join("releases.toml"),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.1.0"

[workspaces.root.releases."1.0.0"]
entries = []

[workspaces.root.releases."1.1.0"]
entries = ["minor-01abc.md"]
"#,
        )
        .unwrap();

        run(dir.path()).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.0.0");
        assert!(!root.releases.contains_key("1.1.0"));
        assert!(root.releases.contains_key("1.0.0"));
    }

    #[test]
    fn revert_single_without_prior_version_errors() {
        let dir = tempfile::tempdir().unwrap();
        let boop_dir = dir.path().join(".boop");
        fs::create_dir_all(&boop_dir).unwrap();
        fs::write(
            boop_dir.join("releases.toml"),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"

[workspaces.root.releases."1.0.0"]
entries = []
"#,
        )
        .unwrap();

        let err = run(dir.path()).unwrap_err();
        assert!(matches!(err, RevertError::NoPriorVersion));

        let manifest = store::read_manifest(dir.path()).unwrap();
        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.0.0");
        assert!(root.releases.contains_key("1.0.0"));
    }

    // -- Nested workspace tests --

    #[test]
    fn revert_nested_workspace_group_apply() {
        let dir = tempfile::tempdir().unwrap();

        // Setup nested tree
        fs::create_dir_all(dir.path().join(".boop")).unwrap();
        fs::write(
            dir.path().join(".boop/releases.toml"),
            "workspaces = [\"typescript\"]\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("typescript/.boop")).unwrap();
        fs::write(
            dir.path().join("typescript/.boop/releases.toml"),
            "workspaces = [\"core\", \"unions\"]\n",
        )
        .unwrap();
        for (path, ver) in [("typescript/core", "1.0.0"), ("typescript/unions", "0.1.0")] {
            fs::create_dir_all(dir.path().join(path).join(".boop")).unwrap();
            fs::write(
                dir.path().join(path).join(".boop/releases.toml"),
                format!("version = \"{ver}\"\n"),
            )
            .unwrap();
        }

        // Add entries
        let cl = dir.path().join("typescript/core/.boop/changelogs");
        fs::create_dir_all(&cl).unwrap();
        fs::write(cl.join("minor-01abc.md"), "## Feature").unwrap();
        let cl2 = dir.path().join("typescript/unions/.boop/changelogs");
        fs::create_dir_all(&cl2).unwrap();
        fs::write(cl2.join("patch-01def.md"), "## Fix").unwrap();

        // Apply with --all on typescript group
        crate::commands::apply::run(dir.path(), None, false, Some("typescript"), true, false)
            .unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(
            manifest.workspaces.get("typescript/core").unwrap().version,
            "1.1.0"
        );
        assert_eq!(
            manifest
                .workspaces
                .get("typescript/unions")
                .unwrap()
                .version,
            "0.1.1"
        );

        // Revert
        run(dir.path()).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(
            manifest.workspaces.get("typescript/core").unwrap().version,
            "1.0.0"
        );
        assert_eq!(
            manifest
                .workspaces
                .get("typescript/unions")
                .unwrap()
                .version,
            "0.1.0"
        );
        assert!(manifest.release_groups.is_empty());
    }
}
