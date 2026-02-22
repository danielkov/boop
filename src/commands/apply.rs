// boop apply — version resolution + changelog assembly

use std::collections::BTreeMap;
use std::path::Path;

use crate::errors::ApplyError;
use crate::store::{self, Release, ReleaseGroup};
use crate::version::{self, parse_kind};

struct WorkspacePlan {
    name: String,
    current: semver::Version,
    next: semver::Version,
    pending: Vec<String>,
}

pub fn run(
    base: &Path,
    pre_tag: Option<Option<&str>>,
    current_mode: bool,
    workspaces: Option<&str>,
    all: bool,
    dry_run: bool,
) -> Result<(), ApplyError> {
    store::ensure_initialized(base)?;

    if current_mode && pre_tag.is_some() {
        return Err(ApplyError::ConflictingCurrentAndPre);
    }

    let mut manifest = store::read_manifest(base)?;
    let workspace_mode = store::is_workspace_mode(base)?;

    // Only perform filesystem migration when not in dry-run mode.
    // Dry-run must not mutate the filesystem.
    if !dry_run {
        store::maybe_migrate_to_multi(base, &manifest)?;
    }

    // Determine target workspace names.
    // In workspace mode without selectors, require explicit -w or --all.
    let targets: Vec<String> = if workspaces.is_none() && !all && workspace_mode {
        return Err(ApplyError::WorkspaceSelectorRequired);
    } else {
        store::resolve_workspace_targets(&manifest, workspaces, all)?
    };

    // Compute plan for each target workspace
    let mut plans: Vec<WorkspacePlan> = Vec::new();

    for name in &targets {
        let all_entries = store::list_entry_filenames_for_workspace(base, &manifest, name)?;
        let pending = store::pending_entries(&manifest, name, &all_entries);

        if pending.is_empty() {
            continue;
        }

        // Separate recognized bump-kind entries from unrecognized ones.
        let (recognized, unrecognized): (Vec<String>, Vec<String>) =
            pending.into_iter().partition(|f| parse_kind(f).is_some());

        if !unrecognized.is_empty() {
            eprintln!(
                "warning: skipping {} unrecognized changelog entries in {}: {}",
                unrecognized.len(),
                name,
                unrecognized.join(", "),
            );
        }

        let pending = recognized;
        if pending.is_empty() {
            continue;
        }

        let ws = manifest
            .workspaces
            .get(name)
            .ok_or(ApplyError::UnknownWorkspace { name: name.clone() })?;

        let current = semver::Version::parse(&ws.version).map_err(|_| {
            crate::errors::VersionError::InvalidVersion {
                input: ws.version.clone(),
            }
        })?;

        let next = if current_mode {
            current.clone()
        } else {
            // SAFETY: pending is non-empty and contains only recognized kinds
            // (filtered above), so highest_bump always returns Some.
            let bump = version::highest_bump(&pending)
                .expect("pending contains only recognized bump kinds");
            version::resolve_next_version(&current, bump, pre_tag)?
        };

        plans.push(WorkspacePlan {
            name: name.clone(),
            current,
            next,
            pending,
        });
    }

    if plans.is_empty() {
        return Err(ApplyError::NoPendingEntries);
    }

    // Dry-run: print plan and return
    if dry_run {
        println!("Planned releases:");
        for plan in &plans {
            if current_mode {
                println!(
                    "  {} {} (merge {} entries into current)",
                    plan.name,
                    plan.current,
                    plan.pending.len()
                );
            } else {
                println!(
                    "  {} {} → {} ({} entries)",
                    plan.name,
                    plan.current,
                    plan.next,
                    plan.pending.len()
                );
            }
            for entry in &plan.pending {
                println!("    - {entry}");
            }
        }
        return Ok(());
    }

    if current_mode {
        eprintln!(
            "warning: --current rewrites release history in place and cannot be reverted with `boop revert`"
        );

        for plan in &plans {
            let ws = manifest.workspaces.get_mut(&plan.name).unwrap();
            let current_version = plan.current.to_string();
            let release = ws
                .releases
                .entry(current_version.clone())
                .or_insert_with(|| Release {
                    entries: Vec::new(),
                    release_group: None,
                });

            let mut added = 0usize;
            for entry in &plan.pending {
                if !release.entries.contains(entry) {
                    release.entries.push(entry.clone());
                    added += 1;
                }
            }

            println!(
                "{}: merged {} entries into {}",
                plan.name, added, current_version
            );
        }

        store::write_manifest(base, &manifest)?;
        return Ok(());
    }

    // Generate one release_group ID for this apply run
    let release_group_id = format!("rg-{}", ulid::Ulid::new().to_string().to_lowercase());

    let mut before: BTreeMap<String, String> = BTreeMap::new();
    let mut after: BTreeMap<String, String> = BTreeMap::new();
    let mut group_workspaces: Vec<String> = Vec::new();

    for plan in &plans {
        before.insert(plan.name.clone(), plan.current.to_string());
        after.insert(plan.name.clone(), plan.next.to_string());
        group_workspaces.push(plan.name.clone());

        let ws = manifest.workspaces.get_mut(&plan.name).unwrap();
        ws.releases.insert(
            plan.next.to_string(),
            Release {
                entries: plan.pending.clone(),
                release_group: Some(release_group_id.clone()),
            },
        );
        ws.version = plan.next.to_string();

        println!("{}: {} → {}", plan.name, plan.current, plan.next);
    }

    manifest.release_groups.push(ReleaseGroup {
        id: release_group_id,
        workspaces: group_workspaces,
        before,
        after,
    });

    store::write_manifest(base, &manifest)?;

    Ok(())
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
    fn apply_single_workspace_no_flags_targets_default() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        add_entry(dir.path(), "root", "minor-01abc.md", "## Feature");

        run(dir.path(), None, false, None, false, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.1.0");
        assert!(root.releases.contains_key("1.1.0"));

        let release = root.releases.get("1.1.0").unwrap();
        assert!(release.release_group.is_some());

        // release_groups should have one entry
        assert_eq!(manifest.release_groups.len(), 1);
        let rg = &manifest.release_groups[0];
        assert_eq!(rg.workspaces, vec!["root"]);
        assert_eq!(rg.before.get("root").unwrap(), "1.0.0");
        assert_eq!(rg.after.get("root").unwrap(), "1.1.0");
    }

    #[test]
    fn apply_multi_workspace_requires_selector_or_all() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".boop")).unwrap();
        fs::write(
            dir.path().join(".boop/releases.toml"),
            r#"
workspaces = [".", "apps/api"]
default_workspace = "."
version = "1.0.0"

[releases."1.0.0"]
entries = []
"#,
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("apps/api/.boop")).unwrap();
        fs::write(
            dir.path().join("apps/api/.boop/releases.toml"),
            r#"
version = "0.1.0"
"#,
        )
        .unwrap();
        add_entry(dir.path(), ".", "minor-01abc.md", "## Feature");

        let err = run(dir.path(), None, false, None, false, false).unwrap_err();
        assert!(matches!(err, ApplyError::WorkspaceSelectorRequired));
    }

    #[test]
    fn apply_workspace_mode_updates_workspace_manifest_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".boop")).unwrap();
        fs::write(
            dir.path().join(".boop/releases.toml"),
            r#"
workspaces = [".", "apps/api"]
default_workspace = "."
version = "1.0.0"

[releases."1.0.0"]
entries = []
"#,
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("apps/api/.boop")).unwrap();
        fs::write(
            dir.path().join("apps/api/.boop/releases.toml"),
            r#"
version = "2.0.0"

[releases."2.0.0"]
entries = []
"#,
        )
        .unwrap();

        add_entry(dir.path(), ".", "minor-01root.md", "## Root feature");
        add_entry(dir.path(), "apps/api", "patch-01api.md", "## API fix");

        run(dir.path(), None, false, None, true, false).unwrap();

        let root_manifest = fs::read_to_string(dir.path().join(".boop/releases.toml")).unwrap();
        let root_toml: toml::Value = toml::from_str(&root_manifest).unwrap();
        let ws = root_toml
            .get("workspaces")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(ws[0].as_str(), Some("."));
        assert_eq!(ws[1].as_str(), Some("apps/api"));
        assert_eq!(
            root_toml.get("version").and_then(|v| v.as_str()),
            Some("1.1.0")
        );

        let api_manifest =
            fs::read_to_string(dir.path().join("apps/api/.boop/releases.toml")).unwrap();
        assert!(api_manifest.contains("version = \"2.0.1\""));
        assert!(api_manifest.contains("patch-01api.md"));

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get(".").unwrap().version, "1.1.0");
        assert_eq!(
            manifest.workspaces.get("apps/api").unwrap().version,
            "2.0.1"
        );
        assert_eq!(manifest.release_groups.len(), 1);
    }

    #[test]
    fn apply_single_workspace_flag() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(
            dir.path(),
            &[("root", ".", "1.0.0"), ("api", "apps/api", "2.0.0")],
            "root",
        );
        add_entry(dir.path(), "api", "patch-01abc.md", "## Fix");

        run(dir.path(), None, false, Some("api"), false, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        let api = manifest.workspaces.get("api").unwrap();
        assert_eq!(api.version, "2.0.1");

        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.0.0");
    }

    #[test]
    fn apply_multi_workspace_flag() {
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

        run(dir.path(), None, false, Some("api,web"), false, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        let api = manifest.workspaces.get("api").unwrap();
        assert_eq!(api.version, "2.1.0");

        let web = manifest.workspaces.get("web").unwrap();
        assert_eq!(web.version, "0.5.1");

        // Shared release_group
        assert_eq!(manifest.release_groups.len(), 1);
        let rg = &manifest.release_groups[0];
        assert!(rg.workspaces.contains(&"api".to_string()));
        assert!(rg.workspaces.contains(&"web".to_string()));

        let api_release = api.releases.get("2.1.0").unwrap();
        let web_release = web.releases.get("0.5.1").unwrap();
        assert_eq!(api_release.release_group, web_release.release_group);

        // root untouched
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.0.0");
    }

    #[test]
    fn apply_all_flag() {
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
        add_entry(dir.path(), "api", "major-01abc.md", "## Breaking");
        add_entry(dir.path(), "web", "patch-01def.md", "## Fix");
        // root has no entries — should be skipped

        run(dir.path(), None, false, None, true, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get("api").unwrap().version, "3.0.0");
        assert_eq!(manifest.workspaces.get("web").unwrap().version, "0.5.1");
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.0.0");

        assert_eq!(manifest.release_groups.len(), 1);
        let rg = &manifest.release_groups[0];
        assert!(rg.workspaces.contains(&"api".to_string()));
        assert!(rg.workspaces.contains(&"web".to_string()));
        assert!(!rg.workspaces.contains(&"root".to_string()));
    }

    #[test]
    fn apply_dry_run_no_writes() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        add_entry(dir.path(), "root", "minor-01abc.md", "## Feature");

        run(dir.path(), None, false, None, false, true).unwrap();

        // Manifest should be unchanged
        let manifest = store::read_manifest(dir.path()).unwrap();
        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.0.0");
        assert!(root.releases.is_empty());
        assert!(manifest.release_groups.is_empty());
    }

    #[test]
    fn apply_workspace_and_all_combined_works() {
        // -w and --all are now compatible: -w specifies scope, --all expands groups
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        add_entry(dir.path(), "root", "minor-01abc.md", "## Feature");

        // -w root --all should work (root is a leaf, --all is just passthrough)
        run(dir.path(), None, false, Some("root"), true, false).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.1.0");
    }

    #[test]
    fn apply_unknown_workspace_error() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");

        let err = run(dir.path(), None, false, Some("nonexistent"), false, false).unwrap_err();
        assert!(
            matches!(err, ApplyError::Store(crate::errors::StoreError::UnknownWorkspace { ref name }) if name == "nonexistent"),
            "expected UnknownWorkspace error, got: {err}"
        );
    }

    #[test]
    fn apply_no_pending_entries_error() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        // No entries added

        let err = run(dir.path(), None, false, None, false, false).unwrap_err();
        assert!(matches!(err, ApplyError::NoPendingEntries));
    }

    #[test]
    fn apply_all_no_pending_anywhere_error() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(
            dir.path(),
            &[("root", ".", "1.0.0"), ("api", "apps/api", "2.0.0")],
            "root",
        );

        let err = run(dir.path(), None, false, None, true, false).unwrap_err();
        assert!(matches!(err, ApplyError::NoPendingEntries));
    }

    #[test]
    fn apply_multi_workspace_skips_no_pending() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(
            dir.path(),
            &[("root", ".", "1.0.0"), ("api", "apps/api", "2.0.0")],
            "root",
        );
        // Only api has entries
        add_entry(dir.path(), "api", "patch-01abc.md", "## Fix");

        run(dir.path(), None, false, Some("root,api"), false, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        // api updated, root untouched
        assert_eq!(manifest.workspaces.get("api").unwrap().version, "2.0.1");
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.0.0");

        // Only api in release group
        let rg = &manifest.release_groups[0];
        assert_eq!(rg.workspaces, vec!["api"]);
    }

    #[test]
    fn apply_with_pre_release_tag() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        add_entry(dir.path(), "root", "minor-01abc.md", "## Feature");

        run(dir.path(), Some(Some("beta")), false, None, false, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.1.0-beta.0");
    }

    #[test]
    fn apply_current_merges_into_existing_release_without_bump() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        fs::write(
            dir.path().join(".boop/releases.toml"),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"

[workspaces.root.releases."1.0.0"]
entries = ["patch-01old.md"]
"#,
        )
        .unwrap();
        store::write_entry_scoped(dir.path(), "root", "patch-01old.md", "## Old", false).unwrap();
        store::write_entry_scoped(dir.path(), "root", "minor-01new.md", "## New", false).unwrap();

        run(dir.path(), None, true, None, false, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.0.0");
        let release = root.releases.get("1.0.0").unwrap();
        assert_eq!(
            release.entries,
            vec!["patch-01old.md".to_string(), "minor-01new.md".to_string()]
        );
        assert!(manifest.release_groups.is_empty());
    }

    #[test]
    fn apply_current_conflicts_with_pre() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        let err = run(dir.path(), Some(Some("beta")), true, None, false, false).unwrap_err();
        assert!(matches!(err, ApplyError::ConflictingCurrentAndPre));
    }

    #[test]
    fn apply_release_group_id_format() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        add_entry(dir.path(), "root", "patch-01abc.md", "## Fix");

        run(dir.path(), None, false, None, false, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        let rg = &manifest.release_groups[0];
        assert!(rg.id.starts_with("rg-"));

        let release = manifest
            .workspaces
            .get("root")
            .unwrap()
            .releases
            .get("1.0.1")
            .unwrap();
        assert_eq!(release.release_group.as_deref(), Some(rg.id.as_str()));
    }

    #[test]
    fn apply_dry_run_with_all_flag() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(
            dir.path(),
            &[("root", ".", "1.0.0"), ("api", "apps/api", "2.0.0")],
            "root",
        );
        add_entry(dir.path(), "root", "minor-01abc.md", "## Root feature");
        add_entry(dir.path(), "api", "patch-01def.md", "## API fix");

        run(dir.path(), None, false, None, true, true).unwrap();

        // Nothing should have been written
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.0.0");
        assert_eq!(manifest.workspaces.get("api").unwrap().version, "2.0.0");
        assert!(manifest.release_groups.is_empty());
    }

    #[test]
    fn apply_legacy_single_workspace_reads_flat_changelogs() {
        // Simulate a legacy repo: single workspace, flat changelogs dir
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        // Write entry in flat layout (not in root/ subdir)
        store::write_entry_scoped(dir.path(), "root", "minor-01abc.md", "## Feature", false)
            .unwrap();

        // Apply should find and process the flat entry
        run(dir.path(), None, false, None, false, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.1.0");
        assert!(root.releases.contains_key("1.1.0"));
    }

    #[test]
    fn apply_migrates_on_second_workspace_then_works() {
        let dir = tempfile::tempdir().unwrap();

        // Start with single workspace + flat changelogs
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        store::write_entry_scoped(
            dir.path(),
            "root",
            "minor-01abc.md",
            "## Root feature",
            false,
        )
        .unwrap();

        // Now add a second workspace to the manifest
        setup_workspace_manifest(
            dir.path(),
            &[("root", ".", "1.0.0"), ("api", "apps/api", "0.1.0")],
            "root",
        );
        // Write an api entry in the scoped dir (as if added after multi mode activated)
        store::write_entry_scoped(dir.path(), "api", "patch-01def.md", "## API fix", true).unwrap();

        // Apply --all should migrate flat root entries, then apply both
        run(dir.path(), None, false, None, true, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.1.0");
        assert_eq!(manifest.workspaces.get("api").unwrap().version, "0.1.1");
    }

    #[test]
    fn apply_dry_run_does_not_migrate_flat_changelogs() {
        // Regression: dry-run must not move files from flat to root/ subdir
        let dir = tempfile::tempdir().unwrap();

        // Multi-workspace manifest with legacy flat changelog for root
        setup_workspace_manifest(
            dir.path(),
            &[("root", ".", "1.0.0"), ("api", "apps/api", "0.1.0")],
            "root",
        );

        // Legacy flat entry for root (no root/ subdir)
        store::write_entry_scoped(dir.path(), "root", "minor-legacy.md", "## Feature", false)
            .unwrap();
        // Scoped entry for api
        store::write_entry_scoped(dir.path(), "api", "patch-api.md", "## API fix", true).unwrap();

        let cl = dir.path().join(".boop/changelogs");

        // Verify pre-conditions: flat layout for root, scoped for api
        assert!(cl.join("minor-legacy.md").exists());
        assert!(!cl.join("root").exists());
        assert!(cl.join("api/patch-api.md").exists());

        // Dry-run should succeed and report both workspaces
        run(dir.path(), None, false, None, true, true).unwrap();

        // File layout must be unchanged — no migration
        assert!(
            cl.join("minor-legacy.md").exists(),
            "flat changelog entry was moved despite dry-run"
        );
        assert!(
            !cl.join("root").exists(),
            "root/ subdir was created despite dry-run"
        );
        assert!(cl.join("api/patch-api.md").exists());

        // Manifest must be unchanged
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.0.0");
        assert_eq!(manifest.workspaces.get("api").unwrap().version, "0.1.0");
        assert!(manifest.release_groups.is_empty());
    }

    #[test]
    fn apply_dry_run_matches_real_apply_when_default_workspace_is_not_root() {
        // Regression: when default_workspace != "root", dry-run must attribute
        // flat legacy entries to "root" (matching real apply's migration),
        // not to default_workspace.
        let dir = tempfile::tempdir().unwrap();

        // Manifest: workspaces root + api, default_workspace = "api"
        setup_workspace_manifest(
            dir.path(),
            &[("root", ".", "1.0.0"), ("api", "apps/api", "0.1.0")],
            "api",
        );

        // Legacy flat entry (belongs to root after migration)
        store::write_entry_scoped(
            dir.path(),
            "root",
            "minor-root.md",
            "## Root feature",
            false,
        )
        .unwrap();
        // Scoped entry for api
        store::write_entry_scoped(dir.path(), "api", "patch-api.md", "## API fix", true).unwrap();

        // Dry-run: capture plan
        // We can't easily capture stdout, so verify indirectly:
        // dry-run should succeed and not modify the manifest
        run(dir.path(), None, false, None, true, true).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get("root").unwrap().version, "1.0.0");
        assert_eq!(manifest.workspaces.get("api").unwrap().version, "0.1.0");
        assert!(manifest.release_groups.is_empty());

        // Now do the real apply on a fresh copy to compare semantics
        let dir2 = tempfile::tempdir().unwrap();
        setup_workspace_manifest(
            dir2.path(),
            &[("root", ".", "1.0.0"), ("api", "apps/api", "0.1.0")],
            "api",
        );
        store::write_entry_scoped(
            dir2.path(),
            "root",
            "minor-root.md",
            "## Root feature",
            false,
        )
        .unwrap();
        store::write_entry_scoped(dir2.path(), "api", "patch-api.md", "## API fix", true).unwrap();

        run(dir2.path(), None, false, None, true, false).unwrap();

        let manifest2 = store::read_manifest(dir2.path()).unwrap();
        // Real apply: root gets the minor bump (flat entry migrated to root/)
        assert_eq!(manifest2.workspaces.get("root").unwrap().version, "1.1.0");
        // Real apply: api gets the patch bump
        assert_eq!(manifest2.workspaces.get("api").unwrap().version, "0.1.1");

        // Both workspaces should appear in the release group
        let rg = &manifest2.release_groups[0];
        assert!(rg.workspaces.contains(&"root".to_string()));
        assert!(rg.workspaces.contains(&"api".to_string()));
    }

    #[test]
    fn apply_unknown_kind_only_no_panic() {
        // Regression: unknown changelog entry kinds must not panic.
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        add_entry(dir.path(), "root", "note-01abc.md", "## Note");

        let err = run(dir.path(), None, false, None, false, false).unwrap_err();
        assert!(
            matches!(err, ApplyError::NoPendingEntries),
            "expected NoPendingEntries, got: {err}"
        );
    }

    #[test]
    fn apply_mixed_known_and_unknown_kinds() {
        // Known entries should apply; unknown entries should be skipped.
        let dir = tempfile::tempdir().unwrap();
        setup_workspace_manifest(dir.path(), &[("root", ".", "1.0.0")], "root");
        add_entry(dir.path(), "root", "minor-01abc.md", "## Feature");
        add_entry(dir.path(), "root", "note-02def.md", "## Note");

        run(dir.path(), None, false, None, false, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.1.0");

        let release = root.releases.get("1.1.0").unwrap();
        // Only the recognized entry should be in the release
        assert_eq!(release.entries, vec!["minor-01abc.md".to_string()]);
    }

    // -- Nested workspace tests --

    fn setup_nested_tree(base: &Path) {
        // root → [typescript, java]
        // typescript → [core, unions]
        // java → [core]
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
        for (path, ver) in [
            ("typescript/core", "1.0.0"),
            ("typescript/unions", "0.1.0"),
            ("java/core", "5.0.0"),
        ] {
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
    }

    fn add_nested_entry(base: &Path, workspace: &str, filename: &str, content: &str) {
        let cl_dir = base.join(workspace).join(".boop/changelogs");
        fs::create_dir_all(&cl_dir).unwrap();
        fs::write(cl_dir.join(filename), content).unwrap();
    }

    #[test]
    fn apply_nested_single_leaf() {
        let dir = tempfile::tempdir().unwrap();
        setup_nested_tree(dir.path());
        add_nested_entry(
            dir.path(),
            "typescript/core",
            "minor-01abc.md",
            "## Feature",
        );

        run(
            dir.path(),
            None,
            false,
            Some("typescript/core"),
            false,
            false,
        )
        .unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(
            manifest.workspaces.get("typescript/core").unwrap().version,
            "1.1.0"
        );
        // Others unchanged
        assert_eq!(
            manifest
                .workspaces
                .get("typescript/unions")
                .unwrap()
                .version,
            "0.1.0"
        );
        assert_eq!(
            manifest.workspaces.get("java/core").unwrap().version,
            "5.0.0"
        );
    }

    #[test]
    fn apply_nested_group_without_all_errors() {
        let dir = tempfile::tempdir().unwrap();
        setup_nested_tree(dir.path());
        add_nested_entry(
            dir.path(),
            "typescript/core",
            "minor-01abc.md",
            "## Feature",
        );

        let err = run(dir.path(), None, false, Some("typescript"), false, false).unwrap_err();
        assert!(
            matches!(
                err,
                ApplyError::Store(crate::errors::StoreError::WorkspaceIsGroup { .. })
            ),
            "expected WorkspaceIsGroup error, got: {err}"
        );
    }

    #[test]
    fn apply_nested_group_with_all() {
        let dir = tempfile::tempdir().unwrap();
        setup_nested_tree(dir.path());
        add_nested_entry(
            dir.path(),
            "typescript/core",
            "minor-01abc.md",
            "## Feature",
        );
        add_nested_entry(dir.path(), "typescript/unions", "patch-01def.md", "## Fix");

        run(dir.path(), None, false, Some("typescript"), true, false).unwrap();

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
        // java unchanged
        assert_eq!(
            manifest.workspaces.get("java/core").unwrap().version,
            "5.0.0"
        );

        // Release group should cover both typescript workspaces
        assert_eq!(manifest.release_groups.len(), 1);
        let rg = &manifest.release_groups[0];
        assert!(rg.workspaces.contains(&"typescript/core".to_string()));
        assert!(rg.workspaces.contains(&"typescript/unions".to_string()));
        assert!(!rg.workspaces.contains(&"java/core".to_string()));
    }

    #[test]
    fn apply_nested_all_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        setup_nested_tree(dir.path());
        add_nested_entry(
            dir.path(),
            "typescript/core",
            "minor-01abc.md",
            "## Feature",
        );
        add_nested_entry(dir.path(), "java/core", "major-01def.md", "## Breaking");

        run(dir.path(), None, false, None, true, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(
            manifest.workspaces.get("typescript/core").unwrap().version,
            "1.1.0"
        );
        assert_eq!(
            manifest.workspaces.get("java/core").unwrap().version,
            "6.0.0"
        );
        // typescript/unions has no entries, should be unchanged
        assert_eq!(
            manifest
                .workspaces
                .get("typescript/unions")
                .unwrap()
                .version,
            "0.1.0"
        );
    }

    #[test]
    fn apply_nested_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        setup_nested_tree(dir.path());
        add_nested_entry(
            dir.path(),
            "typescript/core",
            "minor-01abc.md",
            "## Feature",
        );

        run(dir.path(), None, false, Some("typescript"), true, true).unwrap();

        // Nothing should have changed
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(
            manifest.workspaces.get("typescript/core").unwrap().version,
            "1.0.0"
        );
        assert!(manifest.release_groups.is_empty());
    }

    #[test]
    fn apply_nested_no_w_no_all_errors_in_workspace_mode() {
        let dir = tempfile::tempdir().unwrap();
        setup_nested_tree(dir.path());
        add_nested_entry(
            dir.path(),
            "typescript/core",
            "minor-01abc.md",
            "## Feature",
        );

        let err = run(dir.path(), None, false, None, false, false).unwrap_err();
        assert!(matches!(err, ApplyError::WorkspaceSelectorRequired));
    }
}
