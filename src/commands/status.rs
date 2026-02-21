use std::path::Path;

use crate::errors::{BoopError, StatusError};
use crate::store;

pub fn run(base: &Path, workspace: Option<&str>, omit_empty: bool) -> Result<(), BoopError> {
    store::ensure_initialized(base)?;

    let manifest = store::read_manifest(base)?;
    store::maybe_migrate_to_multi(base, &manifest)?;

    // Determine which workspaces to display.
    // When -w targets a group, expand to all leaves under it.
    let workspace_names: Vec<String> = match workspace {
        Some(name) => {
            store::validate_workspace_name(name)?;
            if manifest.workspaces.contains_key(name) {
                vec![name.to_string()]
            } else if manifest.groups.contains_key(name) {
                store::leaf_workspaces_under(&manifest, name)
            } else {
                return Err(StatusError::UnknownWorkspace {
                    name: name.to_string(),
                }
                .into());
            }
        }
        None => manifest.workspaces.keys().cloned().collect(),
    };

    let mut printed = 0usize;
    for ws_name in &workspace_names {
        let ws = &manifest.workspaces[ws_name.as_str()];
        let all_entries = store::list_entry_filenames_for_workspace(base, &manifest, ws_name)?;
        let pending = store::pending_entries(&manifest, ws_name, &all_entries);

        if omit_empty && pending.is_empty() {
            continue;
        }

        if printed > 0 {
            println!();
        }
        printed += 1;

        println!("{} ({})", ws_name, ws.version);

        if pending.is_empty() {
            println!("  No pending changelog entries.");
        } else {
            println!("  Pending changelog entries:");
            for filename in &pending {
                let kind = filename.split('-').next().unwrap_or("unknown");
                let content = store::read_entry_for_workspace(base, &manifest, ws_name, filename)?;
                let first_line = content
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('#')
                    .trim();
                println!("    {:<6} {}", kind, first_line);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_workspace_project(base: &Path) {
        let boop_dir = base.join(".boop");
        fs::create_dir_all(&boop_dir).unwrap();

        fs::write(
            boop_dir.join("releases.toml"),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.4.0"

[workspaces.api]
path = "apps/api"
version = "2.1.0"

[workspaces.api.releases."2.1.0"]
entries = ["minor-01abc.md"]

[workspaces.web]
path = "apps/web"
version = "0.8.3"
"#,
        )
        .unwrap();

        // Create changelog entries
        let root_dir = boop_dir.join("changelogs/root");
        fs::create_dir_all(&root_dir).unwrap();
        // root has no pending entries

        let api_dir = boop_dir.join("changelogs/api");
        fs::create_dir_all(&api_dir).unwrap();
        fs::write(api_dir.join("minor-01abc.md"), "## Added new endpoint").unwrap();
        fs::write(api_dir.join("patch-01def.md"), "## Fixed auth bug").unwrap();

        let web_dir = boop_dir.join("changelogs/web");
        fs::create_dir_all(&web_dir).unwrap();
        fs::write(web_dir.join("major-01ghi.md"), "## Breaking CSS change").unwrap();
    }

    #[test]
    fn status_all_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_project(dir.path());

        // Should not error
        let result = run(dir.path(), None, false);
        assert!(result.is_ok());
    }

    #[test]
    fn status_single_workspace() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_project(dir.path());

        let result = run(dir.path(), Some("api"), false);
        assert!(result.is_ok());
    }

    #[test]
    fn status_unknown_workspace_errors() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_project(dir.path());

        let result = run(dir.path(), Some("nonexistent"), false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown workspace: nonexistent"));
    }

    #[test]
    fn status_workspace_no_pending() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_project(dir.path());

        // root has no changelog entries at all, so no pending
        let result = run(dir.path(), Some("root"), false);
        assert!(result.is_ok());
    }

    #[test]
    fn status_omit_empty_skips_no_pending() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_project(dir.path());

        // root has no pending entries; with omit_empty it should still succeed
        // but skip root in the output
        let result = run(dir.path(), None, true);
        assert!(result.is_ok());
    }

    #[test]
    fn status_legacy_single_workspace_sees_flat_entries() {
        let dir = tempfile::tempdir().unwrap();
        let boop_dir = dir.path().join(".boop");
        fs::create_dir_all(&boop_dir).unwrap();

        // Single-workspace manifest
        fs::write(
            boop_dir.join("releases.toml"),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"
"#,
        )
        .unwrap();

        // Flat changelogs layout (no root/ subdir)
        let cl = boop_dir.join("changelogs");
        fs::create_dir_all(&cl).unwrap();
        fs::write(cl.join("minor-01abc.md"), "## New feature").unwrap();

        // Status should see the flat entry without requiring root/
        let result = run(dir.path(), None, false);
        assert!(result.is_ok());
    }
}
