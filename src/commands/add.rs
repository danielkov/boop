// boop major/minor/patch entry creation

use std::path::Path;

use crate::errors::AddError;
use crate::store;
use crate::version::BumpKind;

pub fn run(
    base: &Path,
    kind: BumpKind,
    message: &str,
    workspaces: Option<&str>,
    all: bool,
) -> Result<(), AddError> {
    store::ensure_initialized(base)?;

    let manifest = store::read_manifest(base)?;
    store::maybe_migrate_to_multi(base, &manifest)?;

    let targets = store::resolve_workspace_targets(&manifest, workspaces, all)?;

    let ulid = ulid::Ulid::new().to_string().to_lowercase();
    let filename = format!("{kind}-{ulid}.md");

    for name in &targets {
        let path = store::write_entry_for_workspace(base, &manifest, name, &filename, message)?;
        eprintln!("Created {}", path.display());
    }

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
                "[workspaces.{name}]\npath = \"{path}\"\nversion = \"0.1.0\"\n\n"
            ));
        }
        fs::write(dir.join("releases.toml"), toml).unwrap();
    }

    #[test]
    fn default_workspace_when_no_flag() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", "."), ("api", "apps/api")], "root");

        run(dir.path(), BumpKind::Patch, "fix something", None, false).unwrap();

        let entries = store::list_entry_filenames_scoped(dir.path(), "root", true).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].starts_with("patch-"));

        // api should have no entries
        let api_entries = store::list_entry_filenames_scoped(dir.path(), "api", true).unwrap();
        assert!(api_entries.is_empty());
    }

    #[test]
    fn single_workspace_flag() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", "."), ("api", "apps/api")], "root");

        run(dir.path(), BumpKind::Patch, "api fix", Some("api"), false).unwrap();

        let root_entries = store::list_entry_filenames_scoped(dir.path(), "root", true).unwrap();
        assert!(root_entries.is_empty());

        let api_entries = store::list_entry_filenames_scoped(dir.path(), "api", true).unwrap();
        assert_eq!(api_entries.len(), 1);
        assert!(api_entries[0].starts_with("patch-"));
    }

    #[test]
    fn multi_workspace_flag_shares_ulid() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(
            dir.path(),
            &[("root", "."), ("api", "apps/api"), ("web", "apps/web")],
            "root",
        );

        run(
            dir.path(),
            BumpKind::Minor,
            "shared feature",
            Some("api,web"),
            false,
        )
        .unwrap();

        let root_entries = store::list_entry_filenames_scoped(dir.path(), "root", true).unwrap();
        assert!(root_entries.is_empty());

        let api_entries = store::list_entry_filenames_scoped(dir.path(), "api", true).unwrap();
        assert_eq!(api_entries.len(), 1);
        assert!(api_entries[0].starts_with("minor-"));

        let web_entries = store::list_entry_filenames_scoped(dir.path(), "web", true).unwrap();
        assert_eq!(web_entries.len(), 1);
        assert!(web_entries[0].starts_with("minor-"));

        // Same ULID (same filename) across both workspaces
        assert_eq!(api_entries[0], web_entries[0]);
    }

    #[test]
    fn unknown_workspace_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".")], "root");

        let err = run(
            dir.path(),
            BumpKind::Patch,
            "msg",
            Some("nonexistent"),
            false,
        )
        .unwrap_err();
        assert!(
            matches!(err, AddError::Store(crate::errors::StoreError::UnknownWorkspace { ref name }) if name == "nonexistent"),
            "expected UnknownWorkspace error, got: {err}"
        );
    }

    #[test]
    fn multi_workspace_with_one_unknown_errors_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", "."), ("api", "apps/api")], "root");

        let err = run(dir.path(), BumpKind::Patch, "msg", Some("api,bogus"), false).unwrap_err();
        assert!(
            matches!(err, AddError::Store(crate::errors::StoreError::UnknownWorkspace { ref name }) if name == "bogus"),
            "expected UnknownWorkspace error, got: {err}"
        );

        // api should NOT have been written to since validation happens first
        let api_entries = store::list_entry_filenames_scoped(dir.path(), "api", true).unwrap();
        assert!(api_entries.is_empty());
    }

    #[test]
    fn workspace_mode_path_selector_writes_workspace_local_boop_dir() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".boop")).unwrap();
        fs::write(
            dir.path().join(".boop/releases.toml"),
            r#"
workspaces = [".", "apps/api"]
default_workspace = "."
version = "1.0.0"
"#,
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("apps/api/.boop")).unwrap();
        fs::write(
            dir.path().join("apps/api/.boop/releases.toml"),
            r#"
version = "2.0.0"
"#,
        )
        .unwrap();

        run(
            dir.path(),
            BumpKind::Patch,
            "api fix",
            Some("apps/api"),
            false,
        )
        .unwrap();

        let api_entries = store::list_entry_filenames_scoped(dir.path(), "apps/api", true).unwrap();
        assert_eq!(api_entries.len(), 1);
        assert!(api_entries[0].starts_with("patch-"));
    }

    // -- Nested workspace tests --

    fn setup_nested_tree(base: &Path) {
        fs::create_dir_all(base.join(".boop")).unwrap();
        fs::write(
            base.join(".boop/releases.toml"),
            "workspaces = [\"typescript\", \"java\"]\n",
        )
        .unwrap();
        fs::create_dir_all(base.join("typescript/.boop")).unwrap();
        fs::write(
            base.join("typescript/.boop/releases.toml"),
            "workspaces = [\"core\", \"unions\"]\n",
        )
        .unwrap();
        for (path, ver) in [("typescript/core", "1.0.0"), ("typescript/unions", "0.1.0")] {
            fs::create_dir_all(base.join(path).join(".boop")).unwrap();
            fs::write(
                base.join(path).join(".boop/releases.toml"),
                format!("version = \"{ver}\"\n"),
            )
            .unwrap();
        }
        fs::create_dir_all(base.join("java/.boop")).unwrap();
        fs::write(
            base.join("java/.boop/releases.toml"),
            "workspaces = [\"core\"]\n",
        )
        .unwrap();
        fs::create_dir_all(base.join("java/core/.boop")).unwrap();
        fs::write(
            base.join("java/core/.boop/releases.toml"),
            "version = \"5.0.0\"\n",
        )
        .unwrap();
    }

    #[test]
    fn add_nested_leaf_directly() {
        let dir = tempfile::tempdir().unwrap();
        setup_nested_tree(dir.path());

        run(
            dir.path(),
            BumpKind::Patch,
            "fix something",
            Some("typescript/core"),
            false,
        )
        .unwrap();

        let entries = store::list_entry_filenames(dir.path(), "typescript/core").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].starts_with("patch-"));

        // Other workspaces untouched
        let other = store::list_entry_filenames(dir.path(), "typescript/unions").unwrap();
        assert!(other.is_empty());
    }

    #[test]
    fn add_nested_group_without_all_errors() {
        let dir = tempfile::tempdir().unwrap();
        setup_nested_tree(dir.path());

        let err = run(
            dir.path(),
            BumpKind::Patch,
            "msg",
            Some("typescript"),
            false,
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                AddError::Store(crate::errors::StoreError::WorkspaceIsGroup { .. })
            ),
            "expected WorkspaceIsGroup error, got: {err}"
        );
    }

    #[test]
    fn add_nested_group_with_all() {
        let dir = tempfile::tempdir().unwrap();
        setup_nested_tree(dir.path());

        run(
            dir.path(),
            BumpKind::Minor,
            "shared feature",
            Some("typescript"),
            true,
        )
        .unwrap();

        let core_entries = store::list_entry_filenames(dir.path(), "typescript/core").unwrap();
        assert_eq!(core_entries.len(), 1);
        assert!(core_entries[0].starts_with("minor-"));

        let unions_entries = store::list_entry_filenames(dir.path(), "typescript/unions").unwrap();
        assert_eq!(unions_entries.len(), 1);
        assert!(unions_entries[0].starts_with("minor-"));

        // Same ULID across both
        assert_eq!(core_entries[0], unions_entries[0]);

        // java untouched
        let java_entries = store::list_entry_filenames(dir.path(), "java/core").unwrap();
        assert!(java_entries.is_empty());
    }

    #[test]
    fn add_nested_all_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        setup_nested_tree(dir.path());

        run(dir.path(), BumpKind::Major, "breaking change", None, true).unwrap();

        // All 3 leaf workspaces should have entries
        for ws in ["typescript/core", "typescript/unions", "java/core"] {
            let entries = store::list_entry_filenames(dir.path(), ws).unwrap();
            assert_eq!(entries.len(), 1, "expected 1 entry for {ws}");
            assert!(entries[0].starts_with("major-"));
        }
    }
}
