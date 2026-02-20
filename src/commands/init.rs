use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::detect;
use crate::errors::{InitError, StoreError};
use crate::store::{self, Manifest, Release, Workspace};

pub fn run(
    base: &Path,
    version: Option<&str>,
    workspace: Option<&str>,
    name: Option<&str>,
) -> Result<(), InitError> {
    // --name without -w is invalid
    if name.is_some() && workspace.is_none() {
        return Err(InitError::NameWithoutWorkspace);
    }

    match workspace {
        Some(ws_path) => run_workspace_init(base, version, ws_path, name),
        None => run_root_init(base, version),
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
        default_workspace: "root".to_string(),
        workspaces,
        groups: BTreeMap::new(),
        release_groups: Vec::new(),
    };
    store::write_manifest(base, &manifest)?;

    Ok(())
}

fn run_workspace_init(
    base: &Path,
    version: Option<&str>,
    ws_path: &str,
    name: Option<&str>,
) -> Result<(), InitError> {
    // 1. Require .boop/ exists
    if !store::is_initialized(base) {
        return Err(InitError::NotInitialized);
    }

    // 2. Validate name is a single segment (no /)
    if let Some(n) = name
        && (n.is_empty()
            || n.contains('/')
            || !n
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    {
        return Err(InitError::Store(StoreError::InvalidWorkspaceName {
            name: n.to_string(),
        }));
    }

    // Validate ws_path
    store::validate_workspace_name(ws_path)?;

    // 3. Read current manifest
    let mut manifest = store::read_manifest(base)?;

    let segments: Vec<&str> = ws_path.split('/').collect();
    let top_segment = segments[0];

    // 4. Auto-convert root to workspace mode if groups is empty
    if manifest.groups.is_empty() {
        // Move "root" workspace entry to "."
        if let Some(root_ws) = manifest.workspaces.remove("root") {
            manifest.workspaces.insert(
                ".".to_string(),
                Workspace {
                    path: ".".to_string(),
                    name: root_ws.name,
                    version: root_ws.version,
                    releases: root_ws.releases,
                },
            );
        }
        manifest.default_workspace = ".".to_string();
        manifest.groups.insert(
            ".".to_string(),
            vec![".".to_string(), top_segment.to_string()],
        );
    } else {
        // Already in workspace mode — ensure top segment is in root group
        let root_children = manifest.groups.entry(".".to_string()).or_default();
        if !root_children.contains(&top_segment.to_string()) {
            root_children.push(top_segment.to_string());
        }
    }

    // 5. Compute workspace key
    let ws_key = if let Some(n) = name {
        // Replace last path segment with name in the key
        if segments.len() == 1 {
            n.to_string()
        } else {
            let parent = &segments[..segments.len() - 1];
            format!("{}/{}", parent.join("/"), n)
        }
    } else {
        ws_path.to_string()
    };

    // 6. Idempotent: skip if key already exists
    if manifest.workspaces.contains_key(&ws_key) {
        eprintln!("Workspace {} already exists, skipping.", ws_key);
        return Ok(());
    }

    // 7. Create intermediate groups for each path prefix
    for i in 1..segments.len() {
        let prefix = segments[..i].join("/");
        // Check if prefix is an existing leaf workspace
        if manifest.workspaces.contains_key(&prefix)
            || manifest.workspace_key_for_path(&prefix).is_some()
        {
            return Err(InitError::ParentIsLeaf { path: prefix });
        }
        // Create or update group
        if let Some(children) = manifest.groups.get_mut(&prefix) {
            let child = segments[i].to_string();
            if !children.contains(&child) {
                children.push(child);
            }
        } else {
            manifest
                .groups
                .insert(prefix, vec![segments[i].to_string()]);
        }
    }

    // 8. Insert leaf workspace
    let version = resolve_version(&base.join(ws_path), version)?;

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
            path: ws_path.to_string(),
            name: name.map(|n| n.to_string()),
            version,
            releases,
        },
    );

    // 9. Write manifest and create changelogs dir
    store::write_manifest(base, &manifest)?;
    let changelogs_dir = base.join(ws_path).join(".boop/changelogs");
    fs::create_dir_all(&changelogs_dir).map_err(InitError::Io)?;

    // 10. Print confirmation
    if name.is_some() {
        eprintln!("Added workspace {} (path: {})", ws_key, ws_path);
    } else {
        eprintln!("Added workspace {}", ws_key);
    }

    Ok(())
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

    #[test]
    fn creates_boop_dir_and_changelogs() {
        let dir = setup_dir();
        run(dir.path(), None, None, None).unwrap();
        assert!(dir.path().join(".boop").exists());
        assert!(dir.path().join(".boop/changelogs").exists());
        // Single-workspace init should NOT create a root/ subdirectory
        assert!(!dir.path().join(".boop/changelogs/root").exists());
    }

    #[test]
    fn creates_releases_toml_with_default_version() {
        let dir = setup_dir();
        run(dir.path(), None, None, None).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        let ws = manifest.default_workspace().unwrap();
        assert_eq!(ws.version, "0.0.1");
        let baseline = ws.releases.get("0.0.1").unwrap();
        assert!(baseline.entries.is_empty());
    }

    #[test]
    fn creates_releases_toml_with_explicit_version() {
        let dir = setup_dir();
        run(dir.path(), Some("1.2.3"), None, None).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        let ws = manifest.default_workspace().unwrap();
        assert_eq!(ws.version, "1.2.3");
    }

    #[test]
    fn errors_if_already_initialized() {
        let dir = setup_dir();
        run(dir.path(), None, None, None).unwrap();
        let err = run(dir.path(), None, None, None).unwrap_err();
        assert!(matches!(err, InitError::AlreadyInitialized { .. }));
    }

    #[test]
    fn errors_on_invalid_semver() {
        let dir = setup_dir();
        let err = run(dir.path(), Some("not-a-version"), None, None).unwrap_err();
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
        run(dir.path(), None, None, None).unwrap();
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
        run(dir.path(), Some("5.0.0"), None, None).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.default_workspace().unwrap().version, "5.0.0");
    }

    #[test]
    fn falls_back_to_default_when_no_heuristic_match() {
        let dir = setup_dir();
        // No package manifest files → should fall back to 0.0.1
        run(dir.path(), None, None, None).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.default_workspace().unwrap().version, "0.0.1");
    }

    #[test]
    fn init_writes_legacy_manifest_shape_by_default() {
        let dir = setup_dir();
        run(dir.path(), Some("1.2.3"), None, None).unwrap();

        let content = fs::read_to_string(dir.path().join(".boop/releases.toml")).unwrap();
        let value: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(value.get("version").and_then(|v| v.as_str()), Some("1.2.3"));
        assert!(value.get("releases").is_some());
        assert!(value.get("workspaces").is_none());
    }

    // -- Workspace init tests --

    #[test]
    fn workspace_init_auto_converts_root_and_creates_group() {
        let dir = setup_dir();
        run(dir.path(), Some("1.0.0"), None, None).unwrap();

        // Add workspace
        run(dir.path(), None, Some("apps/api"), None).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();

        // Root workspace should be "." now
        assert!(manifest.workspaces.contains_key("."));
        assert_eq!(manifest.workspaces.get(".").unwrap().version, "1.0.0");
        assert_eq!(manifest.default_workspace, ".");

        // New workspace should exist
        assert!(manifest.workspaces.contains_key("apps/api"));
        assert_eq!(
            manifest.workspaces.get("apps/api").unwrap().version,
            "0.0.1"
        );
        assert_eq!(
            manifest.workspaces.get("apps/api").unwrap().path,
            "apps/api"
        );

        // Groups should be set up
        assert!(manifest.groups.contains_key("."));
        let root_children = manifest.groups.get(".").unwrap();
        assert!(root_children.contains(&".".to_string()));
        assert!(root_children.contains(&"apps".to_string()));

        assert!(manifest.groups.contains_key("apps"));
        assert_eq!(
            manifest.groups.get("apps").unwrap(),
            &vec!["api".to_string()]
        );

        // Changelogs dir created
        assert!(dir.path().join("apps/api/.boop/changelogs").exists());
    }

    #[test]
    fn workspace_init_with_name_creates_named_workspace() {
        let dir = setup_dir();
        run(dir.path(), Some("1.0.0"), None, None).unwrap();

        run(dir.path(), None, Some("apps/api"), Some("backend")).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();

        // Key should be "apps/backend" (last segment replaced with name)
        assert!(manifest.workspaces.contains_key("apps/backend"));
        let ws = manifest.workspaces.get("apps/backend").unwrap();
        assert_eq!(ws.path, "apps/api");
        assert_eq!(ws.name.as_deref(), Some("backend"));

        // Path should NOT be a key
        assert!(!manifest.workspaces.contains_key("apps/api"));
    }

    #[test]
    fn workspace_init_errors_without_prior_init() {
        let dir = setup_dir();
        // No init done
        let err = run(dir.path(), None, Some("apps/api"), None).unwrap_err();
        assert!(matches!(err, InitError::NotInitialized));
    }

    #[test]
    fn workspace_init_errors_parent_is_leaf() {
        let dir = setup_dir();
        run(dir.path(), Some("1.0.0"), None, None).unwrap();

        // Add "apps" as a leaf workspace
        run(dir.path(), None, Some("apps"), None).unwrap();

        // Try to add "apps/api" — "apps" is a leaf, not a group
        let err = run(dir.path(), None, Some("apps/api"), None).unwrap_err();
        assert!(matches!(err, InitError::ParentIsLeaf { ref path } if path == "apps"));
    }

    #[test]
    fn workspace_init_idempotent() {
        let dir = setup_dir();
        run(dir.path(), Some("1.0.0"), None, None).unwrap();

        run(dir.path(), None, Some("apps/api"), None).unwrap();
        // Second call should be a no-op
        run(dir.path(), None, Some("apps/api"), None).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.len(), 2); // "." and "apps/api"
    }

    #[test]
    fn name_without_workspace_errors() {
        let dir = setup_dir();
        let err = run(dir.path(), None, None, Some("foo")).unwrap_err();
        assert!(matches!(err, InitError::NameWithoutWorkspace));
    }

    #[test]
    fn workspace_init_single_segment_path() {
        let dir = setup_dir();
        run(dir.path(), Some("1.0.0"), None, None).unwrap();

        run(dir.path(), None, Some("api"), None).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert!(manifest.workspaces.contains_key("api"));
        assert_eq!(manifest.workspaces.get("api").unwrap().path, "api");

        let root_children = manifest.groups.get(".").unwrap();
        assert!(root_children.contains(&"api".to_string()));
    }

    #[test]
    fn workspace_init_single_segment_with_name() {
        let dir = setup_dir();
        run(dir.path(), Some("1.0.0"), None, None).unwrap();

        run(dir.path(), None, Some("api"), Some("backend")).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        // Key = name for single segment
        assert!(manifest.workspaces.contains_key("backend"));
        let ws = manifest.workspaces.get("backend").unwrap();
        assert_eq!(ws.path, "api");
        assert_eq!(ws.name.as_deref(), Some("backend"));
    }

    #[test]
    fn workspace_init_preserves_root_releases() {
        let dir = setup_dir();
        run(dir.path(), Some("1.0.0"), None, None).unwrap();

        // The root init creates a baseline release for 1.0.0
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert!(
            manifest
                .workspaces
                .get("root")
                .unwrap()
                .releases
                .contains_key("1.0.0")
        );

        // Add workspace — root should be converted to "." preserving releases
        run(dir.path(), None, Some("api"), None).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        let root = manifest.workspaces.get(".").unwrap();
        assert!(root.releases.contains_key("1.0.0"));
    }

    #[test]
    fn workspace_init_multiple_workspaces() {
        let dir = setup_dir();
        run(dir.path(), Some("1.0.0"), None, None).unwrap();

        run(dir.path(), None, Some("apps/api"), None).unwrap();
        run(dir.path(), None, Some("apps/web"), None).unwrap();
        run(dir.path(), None, Some("libs/core"), None).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.workspaces.len(), 4); // ".", "apps/api", "apps/web", "libs/core"

        // Root group has ".", "apps", "libs"
        let root_children = manifest.groups.get(".").unwrap();
        assert!(root_children.contains(&".".to_string()));
        assert!(root_children.contains(&"apps".to_string()));
        assert!(root_children.contains(&"libs".to_string()));

        // "apps" group has "api", "web"
        let apps_children = manifest.groups.get("apps").unwrap();
        assert!(apps_children.contains(&"api".to_string()));
        assert!(apps_children.contains(&"web".to_string()));

        // "libs" group has "core"
        assert_eq!(
            manifest.groups.get("libs").unwrap(),
            &vec!["core".to_string()]
        );
    }

    #[test]
    fn workspace_init_round_trip() {
        let dir = setup_dir();
        run(dir.path(), Some("1.0.0"), None, None).unwrap();
        run(dir.path(), None, Some("apps/api"), None).unwrap();

        let manifest = store::read_manifest(dir.path()).unwrap();
        store::write_manifest(dir.path(), &manifest).unwrap();
        let reloaded = store::read_manifest(dir.path()).unwrap();

        assert_eq!(manifest.workspaces.len(), reloaded.workspaces.len());
        for (key, ws) in &manifest.workspaces {
            let rws = reloaded.workspaces.get(key).unwrap();
            assert_eq!(ws.version, rws.version);
            assert_eq!(ws.path, rws.path);
        }
    }
}
