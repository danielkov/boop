use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::detect;
use crate::errors::InitError;
use crate::store::{self, GroupInfo, Manifest, Release, Workspace};

pub fn run(
    base: &Path,
    dir: Option<&str>,
    workspace: bool,
    name: Option<&str>,
    version: Option<&str>,
    default: bool,
) -> Result<(), InitError> {
    match (dir, workspace) {
        (None, false) => run_root_init(base, version),
        (None, true) => run_workspace_root_init(base),
        (Some(d), true) => run_group_init(base, d, name),
        (Some(d), false) => run_leaf_init(base, d, name, version, default),
    }
}

fn run_root_init(base: &Path, version: Option<&str>) -> Result<(), InitError> {
    if store::is_initialized(base) {
        return Err(InitError::AlreadyInitialized {
            path: store::boop_dir(base),
        });
    }

    let version = resolve_version(base, version)?;

    store::create_boop_dir(base)?;
    // Single-workspace init: create flat .boop/changelogs/ (legacy layout)
    store::ensure_changelogs_dir_scoped(base, "root", false)?;

    let mut workspaces = BTreeMap::new();
    let mut root_releases = BTreeMap::new();
    root_releases.insert(
        version.clone(),
        Release {
            entries: Vec::new(),
            release_group: None,
        },
    );
    workspaces.insert(
        "root".to_string(),
        Workspace {
            path: ".".to_string(),
            name: None,
            version,
            releases: root_releases,
        },
    );

    let manifest = Manifest {
        default_workspace: Some("root".to_string()),
        workspaces,
        groups: BTreeMap::new(),
        release_groups: Vec::new(),
    };
    store::write_manifest(base, &manifest)?;

    Ok(())
}

fn run_workspace_root_init(base: &Path) -> Result<(), InitError> {
    if store::is_initialized(base) {
        return Err(InitError::AlreadyInitialized {
            path: store::boop_dir(base),
        });
    }

    store::create_boop_dir(base)?;

    let manifest = Manifest {
        default_workspace: None,
        workspaces: BTreeMap::new(),
        groups: {
            let mut g = BTreeMap::new();
            g.insert(
                ".".to_string(),
                GroupInfo {
                    path: ".".to_string(),
                    children: Vec::new(),
                },
            );
            g
        },
        release_groups: Vec::new(),
    };
    store::write_manifest(base, &manifest)?;

    Ok(())
}

fn run_group_init(base: &Path, dir: &str, name: Option<&str>) -> Result<(), InitError> {
    // 1. Require .boop/ exists
    if !store::is_initialized(base) {
        return Err(InitError::NotInitialized);
    }

    // Validate dir as a valid path
    store::validate_workspace_name(dir)?;

    // 2. Read current manifest
    let mut manifest = store::read_manifest(base)?;

    // 3. Find closest parent group
    let (parent_key, parent_path) =
        find_parent_group(&manifest, dir).ok_or_else(|| InitError::NoParentGroup {
            path: dir.to_string(),
        })?;

    // 4. Compute name: name.unwrap_or(relative_path_from_parent)
    let rel_from_parent = relative_from(&parent_path, dir);
    let group_name = name.unwrap_or(&rel_from_parent).to_string();

    // 5. Validate name
    store::validate_workspace_name(&group_name)?;

    // 6. Check for name collision
    if manifest.groups.contains_key(&group_name) {
        return Err(InitError::NameCollision { name: group_name });
    }
    if manifest.workspaces.contains_key(&group_name) {
        return Err(InitError::NameCollision { name: group_name });
    }

    // 7. Register group
    manifest.groups.insert(
        group_name.clone(),
        GroupInfo {
            path: dir.to_string(),
            children: Vec::new(),
        },
    );

    // 8. Add as child of parent group (the path relative to parent)
    if let Some(parent) = manifest.groups.get_mut(&parent_key) {
        let child_rel = relative_from(&parent.path, dir);
        if !parent.children.contains(&child_rel) {
            parent.children.push(child_rel);
        }
    }

    // 9. Write manifest
    store::write_manifest(base, &manifest)?;

    // 10. The group's .boop/releases.toml is written by write_workspace_mode_manifest,
    //     but also ensure the directory exists
    let group_boop = base.join(dir).join(".boop");
    fs::create_dir_all(&group_boop).map_err(InitError::Io)?;

    eprintln!("Added workspace group {} (path: {})", group_name, dir);

    Ok(())
}

fn run_leaf_init(
    base: &Path,
    dir: &str,
    name: Option<&str>,
    version: Option<&str>,
    default: bool,
) -> Result<(), InitError> {
    // 1. Require .boop/ exists
    if !store::is_initialized(base) {
        return Err(InitError::NotInitialized);
    }

    // Validate dir as a valid path
    store::validate_workspace_name(dir)?;

    // 2. Read current manifest
    let mut manifest = store::read_manifest(base)?;

    // 3. Find closest parent group
    let (parent_key, parent_path) =
        find_parent_group(&manifest, dir).ok_or_else(|| InitError::NoParentGroup {
            path: dir.to_string(),
        })?;

    // 4. Compute name: name.unwrap_or(relative_path_from_parent)
    let rel_from_parent = relative_from(&parent_path, dir);
    let ws_name = name.unwrap_or(&rel_from_parent).to_string();

    // 5. Validate name
    store::validate_workspace_name(&ws_name)?;

    // 6. Build fully-qualified key (parent_group/name) for the in-memory map
    let ws_key = if parent_key == "." {
        ws_name.clone()
    } else {
        format!("{parent_key}/{ws_name}")
    };

    // 7. Check for name collision
    if manifest.workspaces.contains_key(&ws_key) {
        eprintln!("Workspace {} already exists, skipping.", ws_key);
        return Ok(());
    }
    if manifest.groups.contains_key(&ws_key) {
        return Err(InitError::NameCollision { name: ws_key });
    }

    // 8. Resolve version
    let version = resolve_version(&base.join(dir), version)?;

    // 9. Register workspace (key = FQN, name = short name)
    let mut releases = BTreeMap::new();
    releases.insert(
        version.clone(),
        Release {
            entries: Vec::new(),
            release_group: None,
        },
    );

    manifest.workspaces.insert(
        ws_key.clone(),
        Workspace {
            path: dir.to_string(),
            name: Some(ws_name),
            version,
            releases,
        },
    );

    // 10. Add as child of parent group
    if let Some(parent) = manifest.groups.get_mut(&parent_key) {
        let child_rel = relative_from(&parent.path, dir);
        if !parent.children.contains(&child_rel) {
            parent.children.push(child_rel);
        }
    }

    if default {
        manifest.default_workspace = Some(ws_key.clone());
    }

    // 11. Write manifest and create changelogs dir
    store::write_manifest(base, &manifest)?;
    let changelogs_dir = base.join(dir).join(".boop/changelogs");
    fs::create_dir_all(&changelogs_dir).map_err(InitError::Io)?;

    eprintln!("Added workspace {} (path: {})", ws_key, dir);

    Ok(())
}

/// Find the closest ancestor group for a given directory path.
/// Returns (group_key, group_path) of the closest ancestor.
/// Walks dir's parent paths from longest to shortest.
/// Root group (".") is always a valid parent.
fn find_parent_group(manifest: &Manifest, dir: &str) -> Option<(String, String)> {
    // Try progressively shorter prefixes of dir
    let parts: Vec<&str> = dir.split('/').collect();
    for i in (1..parts.len()).rev() {
        let prefix = parts[..i].join("/");
        // Check if any group has this prefix as its path
        if let Some(key) = manifest.group_key_for_path(&prefix) {
            return Some((key.to_string(), prefix));
        }
    }
    // Fall back to root group
    if manifest.groups.contains_key(".") {
        return Some((".".to_string(), ".".to_string()));
    }
    None
}

/// Compute the relative path from `parent` to `child`.
/// e.g. relative_from(".", "changelogs/typescript") => "changelogs/typescript"
/// e.g. relative_from("changelogs/typescript", "changelogs/typescript/core") => "core"
fn relative_from(parent: &str, child: &str) -> String {
    if parent == "." {
        child.to_string()
    } else if let Some(rest) = child.strip_prefix(parent) {
        rest.strip_prefix('/').unwrap_or(rest).to_string()
    } else {
        child.to_string()
    }
}

fn resolve_version(base: &Path, version: Option<&str>) -> Result<String, InitError> {
    match version {
        Some(v) => {
            semver::Version::parse(v).map_err(|_| InitError::InvalidVersion {
                input: v.to_string(),
            })?;
            Ok(v.to_string())
        }
        None => match detect::detect_version(base) {
            Ok(Some(v)) => Ok(v),
            _ => Ok("0.0.1".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // -- Legacy root init tests (boop init) --

    #[test]
    fn creates_boop_dir_and_changelogs() {
        let dir = setup_dir();
        run(dir.path(), None, false, None, None, false).unwrap();
        assert!(dir.path().join(".boop").exists());
        assert!(dir.path().join(".boop/changelogs").exists());
        assert!(!dir.path().join(".boop/changelogs/root").exists());
    }

    #[test]
    fn creates_releases_toml_with_default_version() {
        let dir = setup_dir();
        run(dir.path(), None, false, None, None, false).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        let ws = manifest.default_workspace().unwrap();
        assert_eq!(ws.version, "0.0.1");
        let baseline = ws.releases.get("0.0.1").unwrap();
        assert!(baseline.entries.is_empty());
    }

    #[test]
    fn creates_releases_toml_with_explicit_version() {
        let dir = setup_dir();
        run(dir.path(), None, false, None, Some("1.2.3"), false).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        let ws = manifest.default_workspace().unwrap();
        assert_eq!(ws.version, "1.2.3");
    }

    #[test]
    fn errors_if_already_initialized() {
        let dir = setup_dir();
        run(dir.path(), None, false, None, None, false).unwrap();
        let err = run(dir.path(), None, false, None, None, false).unwrap_err();
        assert!(matches!(err, InitError::AlreadyInitialized { .. }));
    }

    #[test]
    fn errors_on_invalid_semver() {
        let dir = setup_dir();
        let err = run(dir.path(), None, false, None, Some("not-a-version"), false).unwrap_err();
        assert!(matches!(err, InitError::InvalidVersion { .. }));
    }

    #[test]
    fn detects_version_from_cargo_toml() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"3.0.0\"\n",
        )
        .unwrap();
        run(dir.path(), None, false, None, None, false).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.default_workspace().unwrap().version, "3.0.0");
    }

    #[test]
    fn explicit_version_overrides_heuristic() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"3.0.0\"\n",
        )
        .unwrap();
        run(dir.path(), None, false, None, Some("5.0.0"), false).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.default_workspace().unwrap().version, "5.0.0");
    }

    #[test]
    fn falls_back_to_default_when_no_heuristic_match() {
        let dir = setup_dir();
        run(dir.path(), None, false, None, None, false).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.default_workspace().unwrap().version, "0.0.1");
    }

    #[test]
    fn init_writes_legacy_manifest_shape_by_default() {
        let dir = setup_dir();
        run(dir.path(), None, false, None, Some("1.2.3"), false).unwrap();

        let content = fs::read_to_string(dir.path().join(".boop/releases.toml")).unwrap();
        let value: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(value.get("version").and_then(|v| v.as_str()), Some("1.2.3"));
        assert!(value.get("releases").is_some());
        assert!(value.get("workspaces").is_none());
    }

    // -- Workspace root init tests (boop init -w) --

    #[test]
    fn workspace_root_init_creates_group() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert!(manifest.groups.contains_key("."));
        assert!(manifest.workspaces.is_empty());
        assert_eq!(manifest.default_workspace, None);
    }

    #[test]
    fn workspace_root_init_errors_if_already_initialized() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        let err = run(dir.path(), None, true, None, None, false).unwrap_err();
        assert!(matches!(err, InitError::AlreadyInitialized { .. }));
    }

    // -- Group init tests (boop init -w <dir>) --

    #[test]
    fn group_init_creates_named_group() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        run(
            dir.path(),
            Some("changelogs/typescript"),
            true,
            Some("typescript"),
            None,
            false,
        )
        .unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert!(manifest.groups.contains_key("typescript"));
        let group = manifest.groups.get("typescript").unwrap();
        assert_eq!(group.path, "changelogs/typescript");
        assert!(group.children.is_empty());

        // Root should have "changelogs/typescript" as child
        let root = manifest.groups.get(".").unwrap();
        assert!(root.children.contains(&"changelogs/typescript".to_string()));
    }

    #[test]
    fn group_init_default_name_is_relative_path() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        run(
            dir.path(),
            Some("changelogs/typescript"),
            true,
            None,
            None,
            false,
        )
        .unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        // Default name = relative path from root = "changelogs/typescript"
        assert!(manifest.groups.contains_key("changelogs/typescript"));
    }

    #[test]
    fn group_init_errors_without_prior_init() {
        let dir = setup_dir();
        let err = run(dir.path(), Some("foo"), true, None, None, false).unwrap_err();
        assert!(matches!(err, InitError::NotInitialized));
    }

    #[test]
    fn group_init_errors_on_name_collision_with_existing_group() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        run(dir.path(), Some("foo"), true, Some("mygroup"), None, false).unwrap();
        let err = run(dir.path(), Some("bar"), true, Some("mygroup"), None, false).unwrap_err();
        assert!(matches!(err, InitError::NameCollision { .. }));
    }

    // -- Leaf init tests (boop init <dir>) --

    #[test]
    fn leaf_init_creates_workspace_under_closest_group() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        run(
            dir.path(),
            Some("changelogs/typescript"),
            true,
            Some("typescript"),
            None,
            false,
        )
        .unwrap();
        run(
            dir.path(),
            Some("changelogs/typescript/core"),
            false,
            Some("core"),
            None,
            false,
        )
        .unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert!(manifest.workspaces.contains_key("typescript/core"));
        let ws = manifest.workspaces.get("typescript/core").unwrap();
        assert_eq!(ws.path, "changelogs/typescript/core");
        assert_eq!(ws.name.as_deref(), Some("core"));

        // "core" should be child of typescript group
        let ts = manifest.groups.get("typescript").unwrap();
        assert!(ts.children.contains(&"core".to_string()));
    }

    #[test]
    fn leaf_init_default_name_is_last_segment() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        run(dir.path(), Some("typescript"), true, None, None, false).unwrap();
        run(
            dir.path(),
            Some("typescript/core"),
            false,
            None,
            None,
            false,
        )
        .unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        // Default name = relative from parent group ("typescript") = "core"
        // FQN key = parent group key / short name = "typescript/core"
        assert!(manifest.workspaces.contains_key("typescript/core"));
    }

    #[test]
    fn leaf_init_errors_without_parent_group() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        // No group at "nonexistent" exists, and dir "nonexistent/leaf" has
        // no registered ancestor beyond root "."
        // Actually root "." IS a valid parent, so this should work.
        // Let me test a case where there's truly no parent group.
        // Actually, root "." is always a parent. So leaf init under root should work.
        run(dir.path(), Some("leaf"), false, None, None, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert!(manifest.workspaces.contains_key("leaf"));
    }

    #[test]
    fn leaf_init_same_name_is_idempotent() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        run(dir.path(), Some("foo"), false, Some("myws"), None, false).unwrap();
        // Same name already exists — idempotent skip (not an error)
        run(dir.path(), Some("bar"), false, Some("myws"), None, false).unwrap();
        // Still only one workspace
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.len(), 1);
    }

    #[test]
    fn leaf_init_idempotent() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        run(dir.path(), Some("foo"), false, None, None, false).unwrap();
        // Second call should be a no-op
        run(dir.path(), Some("foo"), false, None, None, false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.len(), 1);
    }

    #[test]
    fn leaf_init_with_version() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        run(dir.path(), Some("foo"), false, None, Some("2.0.0"), false).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.get("foo").unwrap().version, "2.0.0");
    }

    #[test]
    fn leaf_init_with_default_flag() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        run(dir.path(), Some("foo"), false, None, None, true).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.default_workspace.as_deref(), Some("foo"));
    }

    // -- Round-trip test --

    #[test]
    fn full_workflow_round_trip() {
        let dir = setup_dir();
        // 1. Create workspace root
        run(dir.path(), None, true, None, None, false).unwrap();
        // 2. Create a group
        run(
            dir.path(),
            Some("changelogs/typescript"),
            true,
            Some("typescript"),
            None,
            false,
        )
        .unwrap();
        // 3. Create a leaf under the group
        run(
            dir.path(),
            Some("changelogs/typescript/core"),
            false,
            Some("core"),
            None,
            false,
        )
        .unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert!(manifest.groups.contains_key("."));
        assert!(manifest.groups.contains_key("typescript"));
        assert!(manifest.workspaces.contains_key("typescript/core"));

        // Write and re-read
        store::write_manifest(dir.path(), &manifest).unwrap();
        let reloaded = store::read_manifest(dir.path()).unwrap();

        assert_eq!(manifest.groups.len(), reloaded.groups.len());
        assert_eq!(manifest.workspaces.len(), reloaded.workspaces.len());
        assert!(reloaded.groups.contains_key("typescript"));
        assert!(reloaded.workspaces.contains_key("typescript/core"));
        assert_eq!(
            reloaded.workspaces.get("typescript/core").unwrap().path,
            "changelogs/typescript/core"
        );
    }

    #[test]
    fn leaf_name_collision_with_group_errors() {
        let dir = setup_dir();
        run(dir.path(), None, true, None, None, false).unwrap();
        run(dir.path(), Some("foo"), true, Some("myname"), None, false).unwrap();
        // Try creating a leaf with the same name
        let err = run(dir.path(), Some("bar"), false, Some("myname"), None, false).unwrap_err();
        assert!(matches!(err, InitError::NameCollision { .. }));
    }

    #[test]
    fn same_name_leaves_in_different_groups_do_not_conflict() {
        let dir = setup_dir();
        // 1. Workspace root
        run(dir.path(), None, true, None, None, false).unwrap();
        // 2. Two separate groups
        run(
            dir.path(),
            Some("packages/frontend"),
            true,
            Some("frontend"),
            None,
            false,
        )
        .unwrap();
        run(
            dir.path(),
            Some("packages/backend"),
            true,
            Some("backend"),
            None,
            false,
        )
        .unwrap();
        // 3. Create leaf "utils" under each group (no explicit name → derives "utils")
        fs::create_dir_all(dir.path().join("packages/frontend/utils")).unwrap();
        fs::create_dir_all(dir.path().join("packages/backend/utils")).unwrap();

        run(
            dir.path(),
            Some("packages/frontend/utils"),
            false,
            None,
            None,
            false,
        )
        .unwrap();
        run(
            dir.path(),
            Some("packages/backend/utils"),
            false,
            None,
            None,
            false,
        )
        .unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        // Both leaves should exist — they are in different groups and not ambiguous
        let frontend_utils = manifest
            .workspaces
            .values()
            .find(|ws| ws.path == "packages/frontend/utils");
        let backend_utils = manifest
            .workspaces
            .values()
            .find(|ws| ws.path == "packages/backend/utils");
        assert!(frontend_utils.is_some(), "frontend/utils leaf should exist");
        assert!(backend_utils.is_some(), "backend/utils leaf should exist");
    }
}
