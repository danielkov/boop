// .boop/ filesystem operations, manifest read/write

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::StoreError;

/// Info about a workspace group (a node that contains children, not a leaf workspace).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupInfo {
    /// Filesystem path relative to project root (e.g., "changelogs/typescript" or ".")
    pub path: String,
    /// On-disk child paths (relative to this group's path)
    pub children: Vec<String>,
}

/// Validates that a workspace name contains only safe characters.
/// In workspace mode this is a path-like selector (e.g. `apps/api` or `.`).
pub fn validate_workspace_name(name: &str) -> Result<(), StoreError> {
    if name.is_empty() || name.contains('\\') {
        return Err(StoreError::InvalidWorkspaceName {
            name: name.to_string(),
        });
    }
    if name == "." {
        return Ok(());
    }
    if name.starts_with('/') || name.ends_with('/') {
        return Err(StoreError::InvalidWorkspaceName {
            name: name.to_string(),
        });
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'/')
    {
        return Err(StoreError::InvalidWorkspaceName {
            name: name.to_string(),
        });
    }
    for segment in name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(StoreError::InvalidWorkspaceName {
                name: name.to_string(),
            });
        }
    }
    Ok(())
}

/// Splits a comma-separated workspace selector, trims whitespace, and
/// validates each name. Returns the list of workspace name strings.
pub fn parse_workspace_csv(csv: &str) -> Result<Vec<String>, StoreError> {
    let names: Vec<String> = csv.split(',').map(|s| s.trim().to_string()).collect();
    for name in &names {
        validate_workspace_name(name)?;
    }
    Ok(names)
}

/// Resolve a single workspace selector using longest-prefix matching:
/// 1. Exact match as workspace name → Some leaf
/// 2. Exact match as group name → Some group (caller decides how to handle)
/// 3. Longest prefix that matches a group name, remainder is child lookup
/// 4. None if nothing matches
fn resolve_selector(manifest: &Manifest, selector: &str) -> Option<ResolvedSelector> {
    // 1. Exact match as workspace name
    if manifest.workspaces.contains_key(selector) {
        return Some(ResolvedSelector::Leaf(selector.to_string()));
    }
    // 2. Exact match as group name
    if manifest.groups.contains_key(selector) {
        return Some(ResolvedSelector::Group(selector.to_string()));
    }
    // 3. Longest-prefix matching: try progressively shorter prefixes
    let parts: Vec<&str> = selector.split('/').collect();
    for i in (1..parts.len()).rev() {
        let prefix = parts[..i].join("/");
        let remainder = parts[i..].join("/");
        if manifest.groups.contains_key(&prefix) {
            // Look up remainder as a workspace name
            if manifest.workspaces.contains_key(&remainder) {
                return Some(ResolvedSelector::Leaf(remainder));
            }
            // Look up remainder as a group name
            if manifest.groups.contains_key(&remainder) {
                return Some(ResolvedSelector::Group(remainder));
            }
            // Try constructing full path from group path + remainder and look up by path
            let group = &manifest.groups[&prefix];
            let child_path = if group.path == "." {
                remainder.clone()
            } else {
                format!("{}/{}", group.path, remainder)
            };
            if let Some(ws_key) = manifest.workspace_key_for_path(&child_path) {
                return Some(ResolvedSelector::Leaf(ws_key.to_string()));
            }
            if let Some(grp_key) = manifest.group_key_for_path(&child_path) {
                return Some(ResolvedSelector::Group(grp_key.to_string()));
            }
        }
    }
    None
}

enum ResolvedSelector {
    Leaf(String),
    Group(String),
}

/// Resolve workspace targets from `-w` and `--all` flags.
/// Returns a list of leaf workspace keys.
///
/// Rules:
/// - Without `--all`: each `-w` name must be a leaf workspace
/// - With `--all` and `-w`: expand groups to their leaf descendants
/// - With `--all` alone: all leaf workspaces
/// - Without either: default workspace (must be a leaf)
pub fn resolve_workspace_targets(
    manifest: &Manifest,
    workspaces: Option<&str>,
    all: bool,
) -> Result<Vec<String>, StoreError> {
    if let Some(csv) = workspaces {
        let names = parse_workspace_csv(csv)?;
        if all {
            let mut resolved = Vec::new();
            for name in &names {
                match resolve_selector(manifest, name) {
                    Some(ResolvedSelector::Leaf(key)) => resolved.push(key),
                    Some(ResolvedSelector::Group(key)) => {
                        resolved.extend(leaf_workspaces_under(manifest, &key));
                    }
                    None => return Err(StoreError::UnknownWorkspace { name: name.clone() }),
                }
            }
            Ok(resolved)
        } else {
            let mut resolved = Vec::new();
            for name in &names {
                match resolve_selector(manifest, name) {
                    Some(ResolvedSelector::Leaf(key)) => resolved.push(key),
                    Some(ResolvedSelector::Group(key)) => {
                        return Err(StoreError::WorkspaceIsGroup { name: key });
                    }
                    None => return Err(StoreError::UnknownWorkspace { name: name.clone() }),
                }
            }
            Ok(resolved)
        }
    } else if all {
        Ok(manifest.workspaces.keys().cloned().collect())
    } else {
        let default = manifest
            .default_workspace
            .as_ref()
            .ok_or(StoreError::NoDefaultWorkspace)?;
        if manifest.groups.contains_key(default) && !manifest.workspaces.contains_key(default) {
            return Err(StoreError::WorkspaceIsGroup {
                name: default.clone(),
            });
        }
        if !manifest.workspaces.contains_key(default) {
            return Err(StoreError::UnknownWorkspace {
                name: default.clone(),
            });
        }
        Ok(vec![default.clone()])
    }
}

/// Get all leaf workspace keys under a group, recursively.
pub fn leaf_workspaces_under(manifest: &Manifest, group_name: &str) -> Vec<String> {
    let mut leaves = Vec::new();
    if let Some(group) = manifest.groups.get(group_name) {
        for child in &group.children {
            let full_path = if group.path == "." {
                child.clone()
            } else {
                format!("{}/{child}", group.path)
            };
            if let Some(key) = manifest.workspace_key_for_path(&full_path) {
                leaves.push(key.to_string());
            } else if let Some(sub_key) = manifest.group_key_for_path(&full_path) {
                leaves.extend(leaf_workspaces_under(manifest, sub_key));
            }
        }
    }
    leaves
}

/// Returns true if the manifest has workspace groups (nested workspaces).
pub fn has_groups(manifest: &Manifest) -> bool {
    // Groups always includes "." for the root; actual nesting means there
    // are groups beyond just root, or root's children include other groups.
    manifest.groups.len() > 1
        || manifest.groups.get(".").is_some_and(|root| {
            root.children.iter().any(|c| {
                let full_path = if root.path == "." {
                    c.clone()
                } else {
                    format!("{}/{c}", root.path)
                };
                manifest.group_key_for_path(&full_path).is_some()
            })
        })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_workspace: Option<String>,
    #[serde(default)]
    pub workspaces: BTreeMap<String, Workspace>,
    /// Workspace groups: maps group name → GroupInfo { path, children }.
    /// `"."` represents the root manifest's children.
    #[serde(skip)]
    pub groups: BTreeMap<String, GroupInfo>,
    #[serde(default)]
    pub release_groups: Vec<ReleaseGroup>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub version: String,
    #[serde(default)]
    pub releases: BTreeMap<String, Release>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Release {
    pub entries: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_group: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReleaseGroup {
    pub id: String,
    pub workspaces: Vec<String>,
    pub before: BTreeMap<String, String>,
    pub after: BTreeMap<String, String>,
}

impl Manifest {
    /// Returns a reference to the default workspace.
    pub fn default_workspace(&self) -> Option<&Workspace> {
        self.default_workspace
            .as_ref()
            .and_then(|key| self.workspaces.get(key))
    }

    /// Returns a mutable reference to the default workspace.
    pub fn default_workspace_mut(&mut self) -> Option<&mut Workspace> {
        let key = self.default_workspace.clone()?;
        self.workspaces.get_mut(&key)
    }

    /// Look up a workspace key by its filesystem path.
    /// Returns `None` if no workspace has that path.
    pub fn workspace_key_for_path(&self, path: &str) -> Option<&str> {
        self.workspaces
            .iter()
            .find(|(_, ws)| ws.path == path)
            .map(|(key, _)| key.as_str())
    }

    /// Look up a group key (name) by its filesystem path.
    /// Returns `None` if no group has that path.
    pub fn group_key_for_path(&self, path: &str) -> Option<&str> {
        self.groups
            .iter()
            .find(|(_, g)| g.path == path)
            .map(|(key, _)| key.as_str())
    }
}

// -- Legacy types for backward-compatible loading --

#[derive(Debug, Deserialize, Serialize)]
struct LegacyManifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub version: String,
    #[serde(default)]
    pub releases: BTreeMap<String, LegacyRelease>,
    #[serde(default)]
    pub release_groups: Vec<ReleaseGroup>,
}

#[derive(Debug, Deserialize, Serialize)]
struct LegacyRelease {
    pub entries: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_group: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct WorkspaceRootManifest {
    pub workspaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub releases: BTreeMap<String, LegacyRelease>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub release_groups: Vec<ReleaseGroup>,
}

pub fn boop_dir(base: &Path) -> PathBuf {
    base.join(".boop")
}

/// Returns the changelogs directory for the given workspace. In single-workspace
/// mode (only "root" exists), changelogs live directly in `.boop/changelogs/`
/// (legacy layout). In multi-workspace mode, each workspace gets its own
/// subdirectory: `.boop/changelogs/<workspace>/`.
pub fn changelogs_dir(base: &Path, workspace: &str) -> Result<PathBuf, StoreError> {
    changelogs_dir_scoped(base, workspace, false)
}

/// Like `changelogs_dir`, but `multi` controls whether we force the
/// per-workspace subdirectory layout.
///
/// Returns an error if the workspace name is invalid or contains characters
/// that could cause path traversal.
pub fn changelogs_dir_scoped(
    base: &Path,
    workspace: &str,
    multi: bool,
) -> Result<PathBuf, StoreError> {
    if is_workspace_mode(base).unwrap_or(false) {
        validate_workspace_name(workspace)?;
        let ws_base = if workspace == "." {
            base.to_path_buf()
        } else {
            base.join(workspace)
        };
        return Ok(ws_base.join(".boop/changelogs"));
    }

    let cl = base.join(".boop/changelogs");
    if multi {
        // Reject any workspace name that could escape changelogs/
        if workspace.is_empty()
            || workspace.contains('/')
            || workspace.contains('\\')
            || workspace == "."
            || workspace == ".."
        {
            return Err(StoreError::InvalidWorkspaceName {
                name: workspace.to_string(),
            });
        }
        Ok(cl.join(workspace))
    } else {
        Ok(cl)
    }
}

/// Returns `true` when the manifest describes more than one workspace.
pub fn is_multi_workspace(manifest: &Manifest) -> bool {
    manifest.workspaces.len() > 1
}

/// Returns `true` when the changelog directory has already been migrated to
/// per-workspace subdirectories (i.e. `.boop/changelogs/root/` exists).
pub fn is_multi_layout(base: &Path) -> bool {
    base.join(".boop/changelogs/root").is_dir()
}

/// If the manifest has multiple workspaces but the changelog directory still
/// uses the flat (legacy) layout, migrate `.md` files from `.boop/changelogs/`
/// into `.boop/changelogs/root/` so that per-workspace subdirectories can
/// coexist.  Returns `true` if a migration was performed.
pub fn maybe_migrate_to_multi(base: &Path, manifest: &Manifest) -> Result<bool, StoreError> {
    if is_workspace_mode(base)? {
        return Ok(false);
    }
    if manifest.workspaces.len() <= 1 {
        return Ok(false);
    }
    // Already migrated if root subdir exists
    let root_sub = base.join(".boop/changelogs/root");
    if root_sub.is_dir() {
        return Ok(false);
    }
    // Flat layout still in place — migrate
    migrate_changelogs_to_multi(base)?;
    Ok(true)
}

pub fn manifest_path(base: &Path) -> PathBuf {
    base.join(".boop/releases.toml")
}

pub fn is_initialized(base: &Path) -> bool {
    boop_dir(base).exists()
}

pub fn ensure_initialized(base: &Path) -> Result<(), StoreError> {
    if !is_initialized(base) {
        return Err(StoreError::NotInitialized);
    }
    Ok(())
}

pub fn create_boop_dir(base: &Path) -> Result<(), StoreError> {
    let dir = boop_dir(base);
    fs::create_dir_all(&dir).map_err(|source| StoreError::Write { path: dir, source })
}

pub fn ensure_changelogs_dir(base: &Path, workspace: &str) -> Result<(), StoreError> {
    ensure_changelogs_dir_scoped(base, workspace, false)
}

pub fn ensure_changelogs_dir_scoped(
    base: &Path,
    workspace: &str,
    multi: bool,
) -> Result<(), StoreError> {
    let dir = changelogs_dir_scoped(base, workspace, multi)?;
    fs::create_dir_all(&dir).map_err(|source| StoreError::Write { path: dir, source })
}

/// Recursively load workspace entries from nested manifests.
/// `parent_path` is the fully qualified filesystem path of the parent ("." for root).
/// `parent_group_key` is the resolved key of the parent group (e.g. "." or "frontend").
/// `children` are the relative child paths from the parent's `workspaces` array.
fn load_workspace_tree(
    base: &Path,
    parent_path: &str,
    parent_group_key: &str,
    children: &[String],
    workspaces: &mut BTreeMap<String, Workspace>,
    groups: &mut BTreeMap<String, GroupInfo>,
) -> Result<(), StoreError> {
    for child_rel in children {
        let full_path = if parent_path == "." {
            child_rel.clone()
        } else {
            format!("{parent_path}/{child_rel}")
        };

        let ws_manifest_path = base.join(&full_path).join(".boop/releases.toml");
        let ws_content =
            fs::read_to_string(&ws_manifest_path).map_err(|source| StoreError::Read {
                path: ws_manifest_path.clone(),
                source,
            })?;
        let ws_value: toml::Value =
            toml::from_str(&ws_content).map_err(|source| StoreError::Parse {
                path: ws_manifest_path.clone(),
                source,
            })?;

        if ws_value.get("workspaces").is_some_and(|v| v.is_array()) {
            // It's a group — recurse
            let sub_root: WorkspaceRootManifest =
                toml::from_str(&ws_content).map_err(|source| StoreError::Parse {
                    path: ws_manifest_path,
                    source,
                })?;
            for sub_child in &sub_root.workspaces {
                validate_workspace_name(sub_child)?;
            }
            // Use name field as group key if it's a valid workspace name,
            // otherwise fall back to full_path for backward compatibility.
            // Qualify with parent group key to build fully-qualified name.
            let group_key = match &sub_root.name {
                Some(n) if validate_workspace_name(n).is_ok() => {
                    if parent_group_key == "." {
                        n.clone()
                    } else {
                        format!("{parent_group_key}/{n}")
                    }
                }
                _ => full_path.clone(),
            };
            groups.insert(
                group_key.clone(),
                GroupInfo {
                    path: full_path.clone(),
                    children: sub_root.workspaces.clone(),
                },
            );
            load_workspace_tree(
                base,
                &full_path,
                &group_key,
                &sub_root.workspaces,
                workspaces,
                groups,
            )?;
        } else {
            // It's a leaf workspace
            let ws_legacy: LegacyManifest =
                toml::from_str(&ws_content).map_err(|source| StoreError::Parse {
                    path: ws_manifest_path,
                    source,
                })?;
            // Use name field as workspace key if it's a valid workspace name,
            // otherwise fall back to full_path for backward compatibility.
            // Qualify with parent group key to build fully-qualified name.
            let key = match &ws_legacy.name {
                Some(n) if validate_workspace_name(n).is_ok() => {
                    if parent_group_key == "." {
                        n.clone()
                    } else {
                        format!("{parent_group_key}/{n}")
                    }
                }
                _ => full_path.clone(),
            };
            workspaces.insert(
                key,
                Workspace {
                    path: full_path,
                    name: ws_legacy.name,
                    version: ws_legacy.version,
                    releases: legacy_releases_to_current(&ws_legacy.releases),
                },
            );
        }
    }
    Ok(())
}

pub fn read_manifest(base: &Path) -> Result<Manifest, StoreError> {
    let path = manifest_path(base);
    let content = fs::read_to_string(&path).map_err(|source| StoreError::Read {
        path: path.clone(),
        source,
    })?;
    let root_value: toml::Value = toml::from_str(&content).map_err(|source| StoreError::Parse {
        path: path.clone(),
        source,
    })?;

    // Workspace mode (Cargo-style): top-level `workspaces = ["..."]`.
    if root_value.get("workspaces").is_some_and(|v| v.is_array()) {
        let root: WorkspaceRootManifest =
            toml::from_str(&content).map_err(|source| StoreError::Parse {
                path: path.clone(),
                source,
            })?;

        let mut workspaces = BTreeMap::new();
        let mut groups = BTreeMap::new();

        // Handle "." workspace (root itself is a leaf)
        if root.workspaces.contains(&".".to_string()) {
            let version = root
                .version
                .clone()
                .ok_or_else(|| StoreError::CorruptManifest {
                    reason: "workspace mode with `.` requires root `version`".to_string(),
                })?;
            let releases = legacy_releases_to_current(&root.releases);
            workspaces.insert(
                ".".to_string(),
                Workspace {
                    path: ".".to_string(),
                    name: root.name.clone(),
                    version,
                    releases,
                },
            );
        }

        // Load non-"." entries recursively
        let non_root: Vec<String> = root
            .workspaces
            .iter()
            .filter(|p| *p != ".")
            .cloned()
            .collect();

        for ws_name in &non_root {
            validate_workspace_name(ws_name)?;
        }

        load_workspace_tree(base, ".", ".", &non_root, &mut workspaces, &mut groups)?;

        // Track root-level group
        groups.insert(
            ".".to_string(),
            GroupInfo {
                path: ".".to_string(),
                children: root.workspaces.clone(),
            },
        );

        let default_workspace: Option<String> = if let Some(default_ws) = root.default_workspace {
            if !default_ws.is_empty() {
                validate_workspace_name(&default_ws)?;
                if !workspaces.contains_key(&default_ws) && !groups.contains_key(&default_ws) {
                    return Err(StoreError::CorruptManifest {
                        reason: format!(
                            "default_workspace {default_ws:?} is not in workspaces list"
                        ),
                    });
                }
                Some(default_ws)
            } else {
                None
            }
        } else {
            None
        };

        return Ok(Manifest {
            default_workspace,
            workspaces,
            groups,
            release_groups: root.release_groups,
        });
    }

    // Try new workspace-aware shape first
    if root_value.get("workspaces").is_some_and(|v| v.is_table()) {
        let manifest: Manifest = toml::from_str(&content).map_err(|source| StoreError::Parse {
            path: path.clone(),
            source,
        })?;
        if let Some(ref dw) = manifest.default_workspace {
            validate_workspace_name(dw)?;
        }
        for name in manifest.workspaces.keys() {
            validate_workspace_name(name)?;
            // Table-shape workspace names become subdirectory names under
            // changelogs/, so slashes are not allowed (unlike workspace-mode
            // where each workspace has its own .boop/ at its path).
            if name.contains('/') {
                return Err(StoreError::InvalidWorkspaceName {
                    name: name.to_string(),
                });
            }
        }
        if manifest
            .default_workspace
            .as_ref()
            .is_some_and(|dw| dw.contains('/'))
        {
            return Err(StoreError::InvalidWorkspaceName {
                name: manifest.default_workspace.clone().unwrap_or_default(),
            });
        }
        return Ok(manifest);
    }

    // Fall back to legacy shape
    let legacy: LegacyManifest =
        toml::from_str(&content).map_err(|source| StoreError::Parse { path, source })?;

    // Convert legacy to workspace-aware manifest
    let releases = legacy_releases_to_current(&legacy.releases);

    let mut workspaces = BTreeMap::new();
    workspaces.insert(
        "root".to_string(),
        Workspace {
            path: ".".to_string(),
            name: legacy.name,
            version: legacy.version,
            releases,
        },
    );

    Ok(Manifest {
        default_workspace: Some("root".to_string()),
        workspaces,
        groups: BTreeMap::new(),
        release_groups: legacy.release_groups,
    })
}

pub fn write_manifest(base: &Path, manifest: &Manifest) -> Result<(), StoreError> {
    // Cargo-style workspace mode uses path-keyed workspace names and writes:
    // - root orchestration manifest at .boop/releases.toml
    // - one legacy workspace manifest per workspace path
    if looks_like_workspace_mode_manifest(manifest) {
        write_workspace_mode_manifest(base, manifest)?;
        return Ok(());
    }

    // Default single-workspace write format stays legacy (RFC001).
    if manifest.workspaces.len() == 1 {
        let ws = manifest
            .default_workspace
            .as_ref()
            .and_then(|dw| manifest.workspaces.get(dw))
            .or_else(|| manifest.workspaces.values().next())
            .ok_or_else(|| StoreError::CorruptManifest {
                reason: "single-workspace manifest has no workspace data".to_string(),
            })?;
        let legacy = LegacyManifest {
            name: ws.name.clone(),
            version: ws.version.clone(),
            releases: current_releases_to_legacy(&ws.releases),
            release_groups: manifest.release_groups.clone(),
        };
        let path = manifest_path(base);
        let content =
            toml::to_string_pretty(&legacy).map_err(|source| StoreError::Serialize { source })?;
        fs::write(&path, content).map_err(|source| StoreError::Write { path, source })?;
        return Ok(());
    }

    // Backward-compatible write for existing workspace-table manifests.
    let path = manifest_path(base);
    let content =
        toml::to_string_pretty(manifest).map_err(|source| StoreError::Serialize { source })?;
    fs::write(&path, content).map_err(|source| StoreError::Write { path, source })
}

pub fn is_workspace_mode(base: &Path) -> Result<bool, StoreError> {
    let path = manifest_path(base);
    let content = fs::read_to_string(&path).map_err(|source| StoreError::Read {
        path: path.clone(),
        source,
    })?;
    let value: toml::Value =
        toml::from_str(&content).map_err(|source| StoreError::Parse { path, source })?;
    Ok(value.get("workspaces").is_some_and(|v| v.is_array()))
}

fn legacy_releases_to_current(
    legacy: &BTreeMap<String, LegacyRelease>,
) -> BTreeMap<String, Release> {
    legacy
        .iter()
        .map(|(ver, lr)| {
            (
                ver.clone(),
                Release {
                    entries: lr.entries.clone(),
                    release_group: lr.release_group.clone(),
                },
            )
        })
        .collect()
}

fn current_releases_to_legacy(
    current: &BTreeMap<String, Release>,
) -> BTreeMap<String, LegacyRelease> {
    current
        .iter()
        .map(|(ver, r)| {
            (
                ver.clone(),
                LegacyRelease {
                    entries: r.entries.clone(),
                    release_group: r.release_group.clone(),
                },
            )
        })
        .collect()
}

fn looks_like_workspace_mode_manifest(manifest: &Manifest) -> bool {
    !manifest.groups.is_empty()
}

fn write_workspace_mode_manifest(base: &Path, manifest: &Manifest) -> Result<(), StoreError> {
    // Validate default_workspace exists as leaf or group when set
    if let Some(ref dw) = manifest.default_workspace
        && !manifest.workspaces.contains_key(dw)
        && !manifest.groups.contains_key(dw)
    {
        return Err(StoreError::CorruptManifest {
            reason: format!("default_workspace {dw:?} is not in workspace map"),
        });
    }

    for (name, ws) in &manifest.workspaces {
        validate_workspace_name(name)?;
        if ws.path != "." {
            validate_workspace_name(&ws.path)?;
        }
    }

    // Determine root-level children from groups or fall back to flat workspace list
    let root_children: Vec<String> = if let Some(root_group) = manifest.groups.get(".") {
        root_group.children.clone()
    } else {
        manifest.workspaces.keys().cloned().collect()
    };

    let mut root = WorkspaceRootManifest {
        workspaces: root_children,
        default_workspace: manifest.default_workspace.clone(),
        name: None,
        version: None,
        releases: BTreeMap::new(),
        release_groups: manifest.release_groups.clone(),
    };

    if let Some(root_ws) = manifest.workspaces.get(".") {
        root.version = Some(root_ws.version.clone());
        root.releases = current_releases_to_legacy(&root_ws.releases);
        root.name = root_ws.name.clone();
    }

    let root_path = manifest_path(base);
    let root_content =
        toml::to_string_pretty(&root).map_err(|source| StoreError::Serialize { source })?;
    fs::write(&root_path, root_content).map_err(|source| StoreError::Write {
        path: root_path.clone(),
        source,
    })?;

    // Write group manifests (intermediate workspace groups)
    for (group_name, group) in &manifest.groups {
        if group_name == "." {
            continue; // Already written as root manifest
        }
        let group_manifest = WorkspaceRootManifest {
            workspaces: group.children.clone(),
            default_workspace: None,
            name: Some(group_name.clone()),
            version: None,
            releases: BTreeMap::new(),
            release_groups: Vec::new(),
        };
        let ws_boop = base.join(&group.path).join(".boop");
        fs::create_dir_all(&ws_boop).map_err(|source| StoreError::Write {
            path: ws_boop.clone(),
            source,
        })?;
        let ws_manifest_path = ws_boop.join("releases.toml");
        let ws_content = toml::to_string_pretty(&group_manifest)
            .map_err(|source| StoreError::Serialize { source })?;
        fs::write(&ws_manifest_path, ws_content).map_err(|source| StoreError::Write {
            path: ws_manifest_path,
            source,
        })?;
    }

    // Write leaf workspace manifests
    for ws in manifest.workspaces.values() {
        if ws.path == "." {
            continue;
        }
        let ws_manifest = LegacyManifest {
            name: ws.name.clone(),
            version: ws.version.clone(),
            releases: current_releases_to_legacy(&ws.releases),
            release_groups: Vec::new(),
        };
        let ws_dir = base.join(&ws.path).join(".boop");
        fs::create_dir_all(&ws_dir).map_err(|source| StoreError::Write {
            path: ws_dir.clone(),
            source,
        })?;
        let ws_manifest_path = ws_dir.join("releases.toml");
        let ws_content = toml::to_string_pretty(&ws_manifest)
            .map_err(|source| StoreError::Serialize { source })?;
        fs::write(&ws_manifest_path, ws_content).map_err(|source| StoreError::Write {
            path: ws_manifest_path,
            source,
        })?;
    }

    Ok(())
}

pub fn write_entry(
    base: &Path,
    workspace: &str,
    filename: &str,
    content: &str,
) -> Result<PathBuf, StoreError> {
    write_entry_scoped(base, workspace, filename, content, false)
}

pub fn write_entry_scoped(
    base: &Path,
    workspace: &str,
    filename: &str,
    content: &str,
    multi: bool,
) -> Result<PathBuf, StoreError> {
    ensure_changelogs_dir_scoped(base, workspace, multi)?;
    let path = changelogs_dir_scoped(base, workspace, multi)?.join(filename);
    fs::write(&path, content).map_err(|source| StoreError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn read_entry(base: &Path, workspace: &str, filename: &str) -> Result<String, StoreError> {
    read_entry_scoped(base, workspace, filename, false)
}

pub fn read_entry_scoped(
    base: &Path,
    workspace: &str,
    filename: &str,
    multi: bool,
) -> Result<String, StoreError> {
    let path = changelogs_dir_scoped(base, workspace, multi)?.join(filename);
    fs::read_to_string(&path).map_err(|source| StoreError::Read { path, source })
}

pub fn list_entry_filenames(base: &Path, workspace: &str) -> Result<Vec<String>, StoreError> {
    list_entry_filenames_scoped(base, workspace, false)
}

pub fn list_entry_filenames_scoped(
    base: &Path,
    workspace: &str,
    multi: bool,
) -> Result<Vec<String>, StoreError> {
    let dir = changelogs_dir_scoped(base, workspace, multi)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&dir).map_err(|source| StoreError::Read {
        path: dir.clone(),
        source,
    })?;

    let mut filenames = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| StoreError::Read {
            path: dir.clone(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".md") {
            filenames.push(name);
        }
    }
    filenames.sort();
    Ok(filenames)
}

/// Returns the changelogs directory for a workspace, using the manifest
/// to resolve the filesystem path (supports named workspaces where key ≠ path).
pub fn changelogs_dir_for_workspace(
    base: &Path,
    manifest: &Manifest,
    workspace_key: &str,
) -> Result<PathBuf, StoreError> {
    if !manifest.groups.is_empty() {
        // Workspace mode — entries live at {ws_path}/.boop/changelogs/
        let ws =
            manifest
                .workspaces
                .get(workspace_key)
                .ok_or_else(|| StoreError::UnknownWorkspace {
                    name: workspace_key.to_string(),
                })?;
        let ws_base = if ws.path == "." {
            base.to_path_buf()
        } else {
            base.join(&ws.path)
        };
        Ok(ws_base.join(".boop/changelogs"))
    } else {
        // Non-workspace mode — delegate to existing scoped logic
        let multi = is_multi_workspace(manifest);
        if multi && workspace_key == "root" && !is_multi_layout(base) {
            // Pre-migration fallback: root entries are still in flat layout
            Ok(base.join(".boop/changelogs"))
        } else {
            changelogs_dir_scoped(base, workspace_key, multi)
        }
    }
}

pub fn write_entry_for_workspace(
    base: &Path,
    manifest: &Manifest,
    workspace_key: &str,
    filename: &str,
    content: &str,
) -> Result<PathBuf, StoreError> {
    let dir = changelogs_dir_for_workspace(base, manifest, workspace_key)?;
    fs::create_dir_all(&dir).map_err(|source| StoreError::Write {
        path: dir.clone(),
        source,
    })?;
    let path = dir.join(filename);
    fs::write(&path, content).map_err(|source| StoreError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn read_entry_for_workspace(
    base: &Path,
    manifest: &Manifest,
    workspace_key: &str,
    filename: &str,
) -> Result<String, StoreError> {
    let dir = changelogs_dir_for_workspace(base, manifest, workspace_key)?;
    let path = dir.join(filename);
    fs::read_to_string(&path).map_err(|source| StoreError::Read { path, source })
}

pub fn list_entry_filenames_for_workspace(
    base: &Path,
    manifest: &Manifest,
    workspace_key: &str,
) -> Result<Vec<String>, StoreError> {
    let dir = changelogs_dir_for_workspace(base, manifest, workspace_key)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&dir).map_err(|source| StoreError::Read {
        path: dir.clone(),
        source,
    })?;

    let mut filenames = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| StoreError::Read {
            path: dir.clone(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".md") {
            filenames.push(name);
        }
    }
    filenames.sort();
    Ok(filenames)
}

/// Migrate legacy flat changelogs into per-workspace subdirectories.
/// Moves all `.md` files from `.boop/changelogs/` into `.boop/changelogs/root/`.
pub fn migrate_changelogs_to_multi(base: &Path) -> Result<(), StoreError> {
    let cl = base.join(".boop/changelogs");
    let root_sub = cl.join("root");
    fs::create_dir_all(&root_sub).map_err(|source| StoreError::Write {
        path: root_sub.clone(),
        source,
    })?;

    let entries = fs::read_dir(&cl).map_err(|source| StoreError::Read {
        path: cl.clone(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| StoreError::Read {
            path: cl.clone(),
            source,
        })?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "md") {
            let filename = entry.file_name();
            let dest = root_sub.join(&filename);
            fs::rename(&path, &dest).map_err(|source| StoreError::Write { path: dest, source })?;
        }
    }

    Ok(())
}

pub fn pending_entries(
    manifest: &Manifest,
    workspace: &str,
    all_entries: &[String],
) -> Vec<String> {
    let ws = match manifest.workspaces.get(workspace) {
        Some(ws) => ws,
        None => return all_entries.to_vec(),
    };
    let referenced: std::collections::HashSet<&str> = ws
        .releases
        .values()
        .flat_map(|r| r.entries.iter())
        .map(|s| s.as_str())
        .collect();

    all_entries
        .iter()
        .filter(|e| !referenced.contains(e.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_raw_manifest(base: &std::path::Path, content: &str) {
        let dir = base.join(".boop");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("releases.toml"), content).unwrap();
    }

    #[test]
    fn load_legacy_manifest_converts_to_workspace_aware() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
version = "1.2.3"

[releases."1.2.3"]
entries = ["minor-01abc.md"]

[releases."1.1.0"]
entries = ["patch-01def.md"]
"#,
        );

        let manifest = read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.default_workspace.as_deref(), Some("root"));

        let ws = manifest.default_workspace().unwrap();
        assert_eq!(ws.path, ".");
        assert_eq!(ws.version, "1.2.3");
        assert_eq!(ws.releases.len(), 2);

        let r = ws.releases.get("1.2.3").unwrap();
        assert_eq!(r.entries, vec!["minor-01abc.md"]);
        assert!(r.release_group.is_none());

        assert!(manifest.release_groups.is_empty());
    }

    #[test]
    fn load_legacy_manifest_empty_releases() {
        let dir = setup_dir();
        write_raw_manifest(dir.path(), "version = \"0.0.1\"\n");

        let manifest = read_manifest(dir.path()).unwrap();
        let ws = manifest.default_workspace().unwrap();
        assert_eq!(ws.version, "0.0.1");
        assert!(ws.releases.is_empty());
    }

    #[test]
    fn load_new_manifest_directly() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "2.0.0"

[workspaces.root.releases."2.0.0"]
entries = ["major-01xyz.md"]
release_group = "rg-01abc"

[workspaces.api]
path = "apps/api"
version = "1.0.0"

[[release_groups]]
id = "rg-01abc"
workspaces = ["root"]
before = { root = "1.9.0" }
after = { root = "2.0.0" }
"#,
        );

        let manifest = read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.default_workspace.as_deref(), Some("root"));
        assert_eq!(manifest.workspaces.len(), 2);

        let root = manifest.workspaces.get("root").unwrap();
        assert_eq!(root.version, "2.0.0");
        let r = root.releases.get("2.0.0").unwrap();
        assert_eq!(r.release_group.as_deref(), Some("rg-01abc"));

        let api = manifest.workspaces.get("api").unwrap();
        assert_eq!(api.path, "apps/api");
        assert_eq!(api.version, "1.0.0");

        assert_eq!(manifest.release_groups.len(), 1);
        assert_eq!(manifest.release_groups[0].id, "rg-01abc");
    }

    #[test]
    fn load_workspace_mode_manifest_reads_workspace_files() {
        let dir = setup_dir();
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
entries = ["minor-01abc.md"]
"#,
        )
        .unwrap();

        let manifest = read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.default_workspace.as_deref(), Some("."));
        assert_eq!(manifest.workspaces.get(".").unwrap().version, "1.0.0");
        assert_eq!(
            manifest.workspaces.get("apps/api").unwrap().version,
            "2.0.0"
        );
        assert_eq!(
            manifest
                .workspaces
                .get("apps/api")
                .unwrap()
                .releases
                .get("2.0.0")
                .unwrap()
                .entries,
            vec!["minor-01abc.md"]
        );
    }

    #[test]
    fn write_workspace_mode_manifest_writes_root_and_workspace_files() {
        let dir = setup_dir();
        fs::create_dir_all(dir.path().join(".boop")).unwrap();

        let mut root_releases = BTreeMap::new();
        root_releases.insert(
            "1.0.0".to_string(),
            Release {
                entries: vec![],
                release_group: None,
            },
        );
        let mut api_releases = BTreeMap::new();
        api_releases.insert(
            "2.0.0".to_string(),
            Release {
                entries: vec!["patch-01abc.md".to_string()],
                release_group: None,
            },
        );

        let mut workspaces = BTreeMap::new();
        workspaces.insert(
            ".".to_string(),
            Workspace {
                path: ".".to_string(),
                name: None,
                version: "1.0.0".to_string(),
                releases: root_releases,
            },
        );
        workspaces.insert(
            "apps/api".to_string(),
            Workspace {
                path: "apps/api".to_string(),
                name: None,
                version: "2.0.0".to_string(),
                releases: api_releases,
            },
        );

        let mut groups = BTreeMap::new();
        groups.insert(
            ".".to_string(),
            GroupInfo {
                path: ".".to_string(),
                children: vec![".".to_string(), "apps".to_string()],
            },
        );
        groups.insert(
            "apps".to_string(),
            GroupInfo {
                path: "apps".to_string(),
                children: vec!["api".to_string()],
            },
        );

        let manifest = Manifest {
            default_workspace: Some(".".to_string()),
            workspaces,
            groups,
            release_groups: Vec::new(),
        };
        write_manifest(dir.path(), &manifest).unwrap();

        let root_content = fs::read_to_string(dir.path().join(".boop/releases.toml")).unwrap();
        let root_toml: toml::Value = toml::from_str(&root_content).unwrap();
        let ws = root_toml
            .get("workspaces")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(ws[0].as_str(), Some("."));
        assert_eq!(ws[1].as_str(), Some("apps"));
        assert_eq!(
            root_toml.get("version").and_then(|v| v.as_str()),
            Some("1.0.0")
        );

        let api_content =
            fs::read_to_string(dir.path().join("apps/api/.boop/releases.toml")).unwrap();
        assert!(api_content.contains("version = \"2.0.0\""));
        assert!(api_content.contains("patch-01abc.md"));

        let reloaded = read_manifest(dir.path()).unwrap();
        assert_eq!(reloaded.workspaces.get(".").unwrap().version, "1.0.0");
        assert_eq!(
            reloaded.workspaces.get("apps/api").unwrap().version,
            "2.0.0"
        );
    }

    #[test]
    fn workspace_mode_ignores_unknown_fields() {
        let dir = setup_dir();
        fs::create_dir_all(dir.path().join(".boop")).unwrap();
        fs::write(
            dir.path().join(".boop/releases.toml"),
            r#"
workspaces = [".", "apps/api"]
default_workspace = "."
version = "1.0.0"
custom_metadata = "hello"

[releases."1.0.0"]
entries = []

[tooling]
channel = "dev"
"#,
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("apps/api/.boop")).unwrap();
        fs::write(
            dir.path().join("apps/api/.boop/releases.toml"),
            r#"
version = "2.0.0"
unknown_field = "ignored"
"#,
        )
        .unwrap();

        let manifest = read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.default_workspace.as_deref(), Some("."));
        assert_eq!(manifest.workspaces.get(".").unwrap().version, "1.0.0");
        assert_eq!(
            manifest.workspaces.get("apps/api").unwrap().version,
            "2.0.0"
        );
    }

    #[test]
    fn round_trip_new_manifest() {
        let dir = setup_dir();
        let boop_dir = dir.path().join(".boop");
        fs::create_dir_all(&boop_dir).unwrap();

        let mut root_releases = BTreeMap::new();
        root_releases.insert(
            "1.0.0".to_string(),
            Release {
                entries: vec!["minor-01abc.md".to_string()],
                release_group: Some("rg-001".to_string()),
            },
        );

        let mut workspaces = BTreeMap::new();
        workspaces.insert(
            "root".to_string(),
            Workspace {
                path: ".".to_string(),
                name: None,
                version: "1.0.0".to_string(),
                releases: root_releases,
            },
        );
        workspaces.insert(
            "web".to_string(),
            Workspace {
                path: "apps/web".to_string(),
                name: None,
                version: "0.1.0".to_string(),
                releases: BTreeMap::new(),
            },
        );

        let mut before = BTreeMap::new();
        before.insert("root".to_string(), "0.9.0".to_string());
        let mut after = BTreeMap::new();
        after.insert("root".to_string(), "1.0.0".to_string());

        let manifest = Manifest {
            default_workspace: Some("root".to_string()),
            workspaces,
            groups: BTreeMap::new(),
            release_groups: vec![ReleaseGroup {
                id: "rg-001".to_string(),
                workspaces: vec!["root".to_string()],
                before,
                after,
            }],
        };

        write_manifest(dir.path(), &manifest).unwrap();
        let loaded = read_manifest(dir.path()).unwrap();

        assert_eq!(loaded.default_workspace.as_deref(), Some("root"));
        assert_eq!(loaded.workspaces.len(), 2);

        let root = loaded.workspaces.get("root").unwrap();
        assert_eq!(root.version, "1.0.0");
        assert_eq!(root.releases.len(), 1);
        let r = root.releases.get("1.0.0").unwrap();
        assert_eq!(r.entries, vec!["minor-01abc.md"]);
        assert_eq!(r.release_group.as_deref(), Some("rg-001"));

        let web = loaded.workspaces.get("web").unwrap();
        assert_eq!(web.path, "apps/web");
        assert_eq!(web.version, "0.1.0");
        assert!(web.releases.is_empty());

        assert_eq!(loaded.release_groups.len(), 1);
        assert_eq!(loaded.release_groups[0].id, "rg-001");
        assert_eq!(
            loaded.release_groups[0].before.get("root").unwrap(),
            "0.9.0"
        );
        assert_eq!(loaded.release_groups[0].after.get("root").unwrap(), "1.0.0");
    }

    #[test]
    fn legacy_manifest_migrates_on_write() {
        let dir = setup_dir();
        write_raw_manifest(dir.path(), "version = \"1.0.0\"\n");

        let manifest = read_manifest(dir.path()).unwrap();
        write_manifest(dir.path(), &manifest).unwrap();

        // Re-read should load as new shape directly
        let reloaded = read_manifest(dir.path()).unwrap();
        assert_eq!(reloaded.default_workspace.as_deref(), Some("root"));
        assert_eq!(reloaded.workspaces.get("root").unwrap().version, "1.0.0");
    }

    #[test]
    fn invalid_toml_returns_parse_error() {
        let dir = setup_dir();
        write_raw_manifest(dir.path(), "this is not valid toml {{{}}}");

        let err = read_manifest(dir.path()).unwrap_err();
        assert!(matches!(err, StoreError::Parse { .. }));
    }

    #[test]
    fn changelogs_dir_single_workspace_is_flat() {
        let base = Path::new("/tmp/test");
        // Default (single-workspace) layout: flat .boop/changelogs/
        assert_eq!(
            changelogs_dir(base, "root").unwrap(),
            base.join(".boop/changelogs")
        );
        assert_eq!(
            changelogs_dir_scoped(base, "root", false).unwrap(),
            base.join(".boop/changelogs")
        );
    }

    #[test]
    fn changelogs_dir_multi_workspace_is_scoped() {
        let base = Path::new("/tmp/test");
        assert_eq!(
            changelogs_dir_scoped(base, "root", true).unwrap(),
            base.join(".boop/changelogs/root")
        );
        assert_eq!(
            changelogs_dir_scoped(base, "api", true).unwrap(),
            base.join(".boop/changelogs/api")
        );
    }

    #[test]
    fn write_and_read_entry_single_workspace() {
        let dir = setup_dir();
        create_boop_dir(dir.path()).unwrap();

        let path = write_entry(dir.path(), "root", "minor-01abc.md", "## Feature").unwrap();
        // Single-workspace: file lives directly in changelogs/
        assert!(path.to_str().unwrap().contains("changelogs/minor-01abc.md"));

        let content = read_entry(dir.path(), "root", "minor-01abc.md").unwrap();
        assert_eq!(content, "## Feature");
    }

    #[test]
    fn write_and_read_entry_multi_workspace() {
        let dir = setup_dir();
        create_boop_dir(dir.path()).unwrap();

        let path =
            write_entry_scoped(dir.path(), "root", "minor-01abc.md", "## Feature", true).unwrap();
        assert!(path.to_str().unwrap().contains("changelogs/root/"));

        let content = read_entry_scoped(dir.path(), "root", "minor-01abc.md", true).unwrap();
        assert_eq!(content, "## Feature");
    }

    #[test]
    fn entries_isolated_between_workspaces() {
        let dir = setup_dir();
        create_boop_dir(dir.path()).unwrap();

        write_entry_scoped(
            dir.path(),
            "root",
            "minor-01abc.md",
            "## Root feature",
            true,
        )
        .unwrap();
        write_entry_scoped(dir.path(), "api", "patch-01def.md", "## API fix", true).unwrap();
        write_entry_scoped(dir.path(), "api", "minor-01ghi.md", "## API feature", true).unwrap();

        let root_entries = list_entry_filenames_scoped(dir.path(), "root", true).unwrap();
        assert_eq!(root_entries, vec!["minor-01abc.md"]);

        let api_entries = list_entry_filenames_scoped(dir.path(), "api", true).unwrap();
        assert_eq!(api_entries, vec!["minor-01ghi.md", "patch-01def.md"]);
    }

    #[test]
    fn list_entry_filenames_empty_for_missing_workspace_dir() {
        let dir = setup_dir();
        create_boop_dir(dir.path()).unwrap();

        // multi=true: looks for .boop/changelogs/nonexistent/, which doesn't exist
        let entries = list_entry_filenames_scoped(dir.path(), "nonexistent", true).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn pending_entries_scoped_to_workspace() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"

[workspaces.root.releases."1.0.0"]
entries = ["minor-01abc.md"]

[workspaces.api]
path = "apps/api"
version = "2.0.0"

[workspaces.api.releases."2.0.0"]
entries = ["patch-01xyz.md"]
"#,
        );

        let manifest = read_manifest(dir.path()).unwrap();

        // root workspace: minor-01abc.md is released, patch-01new.md is pending
        let root_all = vec!["minor-01abc.md".to_string(), "patch-01new.md".to_string()];
        let root_pending = pending_entries(&manifest, "root", &root_all);
        assert_eq!(root_pending, vec!["patch-01new.md"]);

        // api workspace: patch-01xyz.md is released, minor-01new.md is pending
        let api_all = vec!["minor-01new.md".to_string(), "patch-01xyz.md".to_string()];
        let api_pending = pending_entries(&manifest, "api", &api_all);
        assert_eq!(api_pending, vec!["minor-01new.md"]);
    }

    #[test]
    fn pending_entries_unknown_workspace_returns_all() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"
"#,
        );

        let manifest = read_manifest(dir.path()).unwrap();
        let all = vec!["minor-01abc.md".to_string()];
        let pending = pending_entries(&manifest, "unknown", &all);
        assert_eq!(pending, vec!["minor-01abc.md"]);
    }

    #[test]
    fn ensure_changelogs_dir_creates_flat_dir() {
        let dir = setup_dir();
        create_boop_dir(dir.path()).unwrap();
        ensure_changelogs_dir(dir.path(), "root").unwrap();
        assert!(dir.path().join(".boop/changelogs").exists());
        assert!(!dir.path().join(".boop/changelogs/root").exists());
    }

    #[test]
    fn ensure_changelogs_dir_scoped_creates_workspace_subdir() {
        let dir = setup_dir();
        create_boop_dir(dir.path()).unwrap();
        ensure_changelogs_dir_scoped(dir.path(), "web", true).unwrap();
        assert!(dir.path().join(".boop/changelogs/web").exists());
    }

    // -- Legacy compatibility tests --

    #[test]
    fn legacy_repo_single_workspace_reads_flat_changelogs() {
        // Simulates an existing repo with legacy layout:
        // .boop/changelogs/minor-01abc.md (no root/ subdir)
        let dir = setup_dir();
        write_raw_manifest(dir.path(), "version = \"1.0.0\"\n");

        // Write a changelog entry in the flat (legacy) dir
        let cl = dir.path().join(".boop/changelogs");
        fs::create_dir_all(&cl).unwrap();
        fs::write(cl.join("minor-01abc.md"), "## Feature").unwrap();

        let manifest = read_manifest(dir.path()).unwrap();
        // Single workspace → multi=false → reads from flat dir
        let entries = list_entry_filenames_scoped(dir.path(), "root", false).unwrap();
        assert_eq!(entries, vec!["minor-01abc.md"]);

        let pending = pending_entries(&manifest, "root", &entries);
        assert_eq!(pending, vec!["minor-01abc.md"]);
    }

    #[test]
    fn legacy_repo_no_root_subdir_required() {
        let dir = setup_dir();
        write_raw_manifest(dir.path(), "version = \"1.0.0\"\n");

        let cl = dir.path().join(".boop/changelogs");
        fs::create_dir_all(&cl).unwrap();

        // No .boop/changelogs/root/ exists
        assert!(!dir.path().join(".boop/changelogs/root").exists());

        let _manifest = read_manifest(dir.path()).unwrap();
        // Should still work — reads from flat dir
        let entries = list_entry_filenames_scoped(dir.path(), "root", false).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn migrate_changelogs_moves_md_files_to_root_subdir() {
        let dir = setup_dir();
        create_boop_dir(dir.path()).unwrap();

        let cl = dir.path().join(".boop/changelogs");
        fs::create_dir_all(&cl).unwrap();
        fs::write(cl.join("minor-01abc.md"), "## Feature").unwrap();
        fs::write(cl.join("patch-01def.md"), "## Fix").unwrap();

        migrate_changelogs_to_multi(dir.path()).unwrap();

        // Files should now be in root/ subdir
        assert!(cl.join("root/minor-01abc.md").exists());
        assert!(cl.join("root/patch-01def.md").exists());
        // And gone from the flat dir
        assert!(!cl.join("minor-01abc.md").exists());
        assert!(!cl.join("patch-01def.md").exists());
    }

    #[test]
    fn maybe_migrate_noop_for_single_workspace() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"
"#,
        );

        let cl = dir.path().join(".boop/changelogs");
        fs::create_dir_all(&cl).unwrap();
        fs::write(cl.join("minor-01abc.md"), "## Feature").unwrap();

        let manifest = read_manifest(dir.path()).unwrap();
        let migrated = maybe_migrate_to_multi(dir.path(), &manifest).unwrap();
        assert!(!migrated);

        // File should remain in flat dir
        assert!(cl.join("minor-01abc.md").exists());
        assert!(!cl.join("root").exists());
    }

    #[test]
    fn maybe_migrate_triggers_on_second_workspace() {
        let dir = setup_dir();
        // Start with a single-workspace manifest and flat changelogs
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"
"#,
        );

        let cl = dir.path().join(".boop/changelogs");
        fs::create_dir_all(&cl).unwrap();
        fs::write(cl.join("minor-01abc.md"), "## Feature").unwrap();
        fs::write(cl.join("patch-01def.md"), "## Fix").unwrap();

        // Now update the manifest to add a second workspace
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"

[workspaces.api]
path = "apps/api"
version = "0.1.0"
"#,
        );

        let manifest = read_manifest(dir.path()).unwrap();
        let migrated = maybe_migrate_to_multi(dir.path(), &manifest).unwrap();
        assert!(migrated);

        // Files should now be in root/ subdir
        assert!(cl.join("root/minor-01abc.md").exists());
        assert!(cl.join("root/patch-01def.md").exists());
        // And gone from the flat dir
        assert!(!cl.join("minor-01abc.md").exists());
        assert!(!cl.join("patch-01def.md").exists());
    }

    #[test]
    fn maybe_migrate_idempotent_after_migration() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"

[workspaces.api]
path = "apps/api"
version = "0.1.0"
"#,
        );

        let cl = dir.path().join(".boop/changelogs");
        fs::create_dir_all(&cl).unwrap();
        fs::write(cl.join("minor-01abc.md"), "## Feature").unwrap();

        let manifest = read_manifest(dir.path()).unwrap();

        // First migration
        let migrated = maybe_migrate_to_multi(dir.path(), &manifest).unwrap();
        assert!(migrated);

        // Second call should be a no-op (root/ subdir already exists)
        let migrated = maybe_migrate_to_multi(dir.path(), &manifest).unwrap();
        assert!(!migrated);

        // File still in root/
        assert!(cl.join("root/minor-01abc.md").exists());
    }

    #[test]
    fn migrated_entries_remain_pending() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"
"#,
        );

        let cl = dir.path().join(".boop/changelogs");
        fs::create_dir_all(&cl).unwrap();
        fs::write(cl.join("minor-01abc.md"), "## Feature").unwrap();

        // Verify entry is pending in flat layout
        let manifest = read_manifest(dir.path()).unwrap();
        let entries = list_entry_filenames_scoped(dir.path(), "root", false).unwrap();
        let pending = pending_entries(&manifest, "root", &entries);
        assert_eq!(pending, vec!["minor-01abc.md"]);

        // Add second workspace and migrate
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"

[workspaces.api]
path = "apps/api"
version = "0.1.0"
"#,
        );

        let manifest = read_manifest(dir.path()).unwrap();
        maybe_migrate_to_multi(dir.path(), &manifest).unwrap();

        // Entry should still be pending in multi layout
        let entries = list_entry_filenames_scoped(dir.path(), "root", true).unwrap();
        let pending = pending_entries(&manifest, "root", &entries);
        assert_eq!(pending, vec!["minor-01abc.md"]);
    }

    #[test]
    fn migrated_released_entries_stay_released() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"

[workspaces.root.releases."1.0.0"]
entries = ["minor-01abc.md"]
"#,
        );

        let cl = dir.path().join(".boop/changelogs");
        fs::create_dir_all(&cl).unwrap();
        fs::write(cl.join("minor-01abc.md"), "## Feature").unwrap();
        // Also a pending entry
        fs::write(cl.join("patch-01def.md"), "## Fix").unwrap();

        // Add second workspace and migrate
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"

[workspaces.root.releases."1.0.0"]
entries = ["minor-01abc.md"]

[workspaces.api]
path = "apps/api"
version = "0.1.0"
"#,
        );

        let manifest = read_manifest(dir.path()).unwrap();
        maybe_migrate_to_multi(dir.path(), &manifest).unwrap();

        let entries = list_entry_filenames_scoped(dir.path(), "root", true).unwrap();
        assert_eq!(entries, vec!["minor-01abc.md", "patch-01def.md"]);

        let pending = pending_entries(&manifest, "root", &entries);
        // Only patch-01def.md should be pending; minor-01abc.md is released
        assert_eq!(pending, vec!["patch-01def.md"]);
    }

    // -- Workspace name validation tests --

    #[test]
    fn validate_workspace_name_valid() {
        assert!(validate_workspace_name("root").is_ok());
        assert!(validate_workspace_name("api").is_ok());
        assert!(validate_workspace_name(".").is_ok());
        assert!(validate_workspace_name("apps/api").is_ok());
        assert!(validate_workspace_name("my-app").is_ok());
        assert!(validate_workspace_name("my_app").is_ok());
        assert!(validate_workspace_name("App123").is_ok());
        assert!(validate_workspace_name("a").is_ok());
    }

    #[test]
    fn validate_workspace_name_rejects_empty() {
        let err = validate_workspace_name("").unwrap_err();
        assert!(matches!(err, StoreError::InvalidWorkspaceName { .. }));
    }

    #[test]
    fn validate_workspace_name_rejects_path_traversal() {
        let err = validate_workspace_name("../../tmp/pwn").unwrap_err();
        assert!(matches!(err, StoreError::InvalidWorkspaceName { .. }));
    }

    #[test]
    fn validate_workspace_name_rejects_dot_segments() {
        assert!(validate_workspace_name("..").is_err());
        assert!(validate_workspace_name("foo/../bar").is_err());
        assert!(validate_workspace_name("foo/./bar").is_err());
        assert!(validate_workspace_name("foo.bar").is_err());
    }

    #[test]
    fn validate_workspace_name_handles_slashes() {
        assert!(validate_workspace_name("foo/bar").is_ok());
        assert!(validate_workspace_name("foo\\bar").is_err());
        assert!(validate_workspace_name("foo//bar").is_err());
    }

    #[test]
    fn validate_workspace_name_rejects_absolute_path() {
        assert!(validate_workspace_name("/etc/passwd").is_err());
    }

    #[test]
    fn validate_workspace_name_rejects_spaces_and_special_chars() {
        assert!(validate_workspace_name("foo bar").is_err());
        assert!(validate_workspace_name("foo@bar").is_err());
        assert!(validate_workspace_name("foo:bar").is_err());
    }

    #[test]
    fn parse_workspace_csv_valid() {
        let names = parse_workspace_csv("api,apps/web,.").unwrap();
        assert_eq!(names, vec!["api", "apps/web", "."]);
    }

    #[test]
    fn parse_workspace_csv_trims_whitespace() {
        let names = parse_workspace_csv("api , web").unwrap();
        assert_eq!(names, vec!["api", "web"]);
    }

    #[test]
    fn parse_workspace_csv_rejects_traversal() {
        let err = parse_workspace_csv("api,../../etc").unwrap_err();
        assert!(matches!(err, StoreError::InvalidWorkspaceName { .. }));
    }

    #[test]
    fn manifest_rejects_traversal_workspace_name() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"

[workspaces."../../tmp/pwn"]
path = "."
version = "0.1.0"
"#,
        );

        let err = read_manifest(dir.path()).unwrap_err();
        assert!(matches!(err, StoreError::InvalidWorkspaceName { .. }));
    }

    #[test]
    fn manifest_rejects_traversal_default_workspace() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "../escape"

[workspaces."../escape"]
path = "."
version = "1.0.0"
"#,
        );

        let err = read_manifest(dir.path()).unwrap_err();
        assert!(matches!(err, StoreError::InvalidWorkspaceName { .. }));
    }

    #[test]
    fn changelogs_dir_scoped_errors_on_traversal() {
        let base = Path::new("/tmp/test");
        // This should return an error because "../escape" would escape the changelogs dir
        let err = changelogs_dir_scoped(base, "../escape", true).unwrap_err();
        assert!(matches!(err, StoreError::InvalidWorkspaceName { .. }));
    }

    #[test]
    fn changelogs_dir_scoped_errors_on_slash_in_multi_mode() {
        let base = Path::new("/tmp/test");
        // Workspace names with slashes are invalid in non-workspace multi mode
        let err = changelogs_dir_scoped(base, "apps/api", true).unwrap_err();
        assert!(matches!(err, StoreError::InvalidWorkspaceName { .. }));
    }

    // -- Regression tests: slash-containing workspace names in table manifests --

    #[test]
    fn table_manifest_rejects_slash_workspace_name() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.0.0"

[workspaces."apps/api"]
path = "apps/api"
version = "0.1.0"
"#,
        );

        let err = read_manifest(dir.path()).unwrap_err();
        assert!(matches!(err, StoreError::InvalidWorkspaceName { .. }));
    }

    #[test]
    fn table_manifest_rejects_slash_default_workspace() {
        let dir = setup_dir();
        write_raw_manifest(
            dir.path(),
            r#"
default_workspace = "apps/api"

[workspaces."apps/api"]
path = "apps/api"
version = "0.1.0"
"#,
        );

        let err = read_manifest(dir.path()).unwrap_err();
        assert!(matches!(err, StoreError::InvalidWorkspaceName { .. }));
    }

    #[test]
    fn workspace_mode_allows_slash_names() {
        // Workspace mode (array style) correctly allows path-like names
        let dir = setup_dir();
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
            "version = \"0.1.0\"\n",
        )
        .unwrap();

        // Should succeed — workspace mode supports path-like names
        let manifest = read_manifest(dir.path()).unwrap();
        assert!(manifest.workspaces.contains_key("apps/api"));
    }

    // -- Nested workspace tests --

    /// Helper: set up a nested workspace tree on disk.
    /// root → [typescript, java]
    /// typescript → [core, unions]
    /// java → [core, serialization]
    fn setup_nested_workspace_tree(base: &std::path::Path) {
        // Root manifest
        fs::create_dir_all(base.join(".boop")).unwrap();
        fs::write(
            base.join(".boop/releases.toml"),
            "workspaces = [\"typescript\", \"java\"]\n",
        )
        .unwrap();

        // typescript group
        fs::create_dir_all(base.join("typescript/.boop")).unwrap();
        fs::write(
            base.join("typescript/.boop/releases.toml"),
            "workspaces = [\"core\", \"unions\"]\n",
        )
        .unwrap();

        // typescript/core leaf
        fs::create_dir_all(base.join("typescript/core/.boop")).unwrap();
        fs::write(
            base.join("typescript/core/.boop/releases.toml"),
            "version = \"1.2.3\"\n",
        )
        .unwrap();

        // typescript/unions leaf
        fs::create_dir_all(base.join("typescript/unions/.boop")).unwrap();
        fs::write(
            base.join("typescript/unions/.boop/releases.toml"),
            "version = \"0.3.2\"\n",
        )
        .unwrap();

        // java group
        fs::create_dir_all(base.join("java/.boop")).unwrap();
        fs::write(
            base.join("java/.boop/releases.toml"),
            "workspaces = [\"core\", \"serialization\"]\n",
        )
        .unwrap();

        // java/core leaf
        fs::create_dir_all(base.join("java/core/.boop")).unwrap();
        fs::write(
            base.join("java/core/.boop/releases.toml"),
            "version = \"5.3.2\"\n",
        )
        .unwrap();

        // java/serialization leaf
        fs::create_dir_all(base.join("java/serialization/.boop")).unwrap();
        fs::write(
            base.join("java/serialization/.boop/releases.toml"),
            "version = \"1.2.3\"\n",
        )
        .unwrap();
    }

    #[test]
    fn nested_workspace_loads_all_leaves() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());

        let manifest = read_manifest(dir.path()).unwrap();

        // Should have 4 leaf workspaces
        assert_eq!(manifest.workspaces.len(), 4);
        assert_eq!(
            manifest.workspaces.get("typescript/core").unwrap().version,
            "1.2.3"
        );
        assert_eq!(
            manifest
                .workspaces
                .get("typescript/unions")
                .unwrap()
                .version,
            "0.3.2"
        );
        assert_eq!(
            manifest.workspaces.get("java/core").unwrap().version,
            "5.3.2"
        );
        assert_eq!(
            manifest
                .workspaces
                .get("java/serialization")
                .unwrap()
                .version,
            "1.2.3"
        );
    }

    #[test]
    fn nested_workspace_tracks_groups() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());

        let manifest = read_manifest(dir.path()).unwrap();

        // Should have 3 groups: ".", "typescript", "java"
        assert_eq!(manifest.groups.len(), 3);
        assert_eq!(
            manifest.groups.get(".").unwrap().children,
            vec!["typescript".to_string(), "java".to_string()]
        );
        assert_eq!(manifest.groups.get(".").unwrap().path, ".");
        assert_eq!(
            manifest.groups.get("typescript").unwrap().children,
            vec!["core".to_string(), "unions".to_string()]
        );
        assert_eq!(
            manifest.groups.get("typescript").unwrap().path,
            "typescript"
        );
        assert_eq!(
            manifest.groups.get("java").unwrap().children,
            vec!["core".to_string(), "serialization".to_string()]
        );
        assert_eq!(manifest.groups.get("java").unwrap().path, "java");
    }

    #[test]
    fn nested_workspace_default_workspace_is_none() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());

        let manifest = read_manifest(dir.path()).unwrap();

        // No default_workspace specified — should be None
        assert_eq!(manifest.default_workspace, None);
    }

    #[test]
    fn nested_workspace_explicit_default_workspace() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());
        // Overwrite root manifest with explicit default
        fs::write(
            dir.path().join(".boop/releases.toml"),
            "workspaces = [\"typescript\", \"java\"]\ndefault_workspace = \"typescript/core\"\n",
        )
        .unwrap();

        let manifest = read_manifest(dir.path()).unwrap();
        assert_eq!(
            manifest.default_workspace.as_deref(),
            Some("typescript/core")
        );
    }

    #[test]
    fn nested_workspace_leaf_workspaces_under_group() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());

        let manifest = read_manifest(dir.path()).unwrap();

        let ts_leaves = leaf_workspaces_under(&manifest, "typescript");
        assert_eq!(ts_leaves, vec!["typescript/core", "typescript/unions"]);

        let java_leaves = leaf_workspaces_under(&manifest, "java");
        assert_eq!(java_leaves, vec!["java/core", "java/serialization"]);

        let all_leaves = leaf_workspaces_under(&manifest, ".");
        assert_eq!(all_leaves.len(), 4);
    }

    #[test]
    fn nested_workspace_resolve_targets_leaf() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());
        let manifest = read_manifest(dir.path()).unwrap();

        // Targeting a leaf directly works
        let targets = resolve_workspace_targets(&manifest, Some("typescript/core"), false).unwrap();
        assert_eq!(targets, vec!["typescript/core"]);
    }

    #[test]
    fn nested_workspace_resolve_targets_group_without_all_errors() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());
        let manifest = read_manifest(dir.path()).unwrap();

        // Targeting a group without --all should error
        let err = resolve_workspace_targets(&manifest, Some("typescript"), false).unwrap_err();
        assert!(matches!(err, StoreError::WorkspaceIsGroup { .. }));
    }

    #[test]
    fn nested_workspace_resolve_targets_group_with_all() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());
        let manifest = read_manifest(dir.path()).unwrap();

        // Targeting a group with --all should expand to leaves
        let targets = resolve_workspace_targets(&manifest, Some("typescript"), true).unwrap();
        assert_eq!(targets, vec!["typescript/core", "typescript/unions"]);
    }

    #[test]
    fn nested_workspace_resolve_targets_multiple_groups_with_all() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());
        let manifest = read_manifest(dir.path()).unwrap();

        let targets = resolve_workspace_targets(&manifest, Some("typescript,java"), true).unwrap();
        assert_eq!(targets.len(), 4);
        assert!(targets.contains(&"typescript/core".to_string()));
        assert!(targets.contains(&"java/serialization".to_string()));
    }

    #[test]
    fn nested_workspace_resolve_targets_all_without_w() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());
        let manifest = read_manifest(dir.path()).unwrap();

        // --all without -w = all leaves
        let targets = resolve_workspace_targets(&manifest, None, true).unwrap();
        assert_eq!(targets.len(), 4);
    }

    #[test]
    fn nested_workspace_resolve_targets_unknown_errors() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());
        let manifest = read_manifest(dir.path()).unwrap();

        let err = resolve_workspace_targets(&manifest, Some("nonexistent"), false).unwrap_err();
        assert!(matches!(err, StoreError::UnknownWorkspace { .. }));
    }

    #[test]
    fn nested_workspace_round_trip() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());

        let manifest = read_manifest(dir.path()).unwrap();
        write_manifest(dir.path(), &manifest).unwrap();
        let reloaded = read_manifest(dir.path()).unwrap();

        assert_eq!(manifest.workspaces.len(), reloaded.workspaces.len());
        assert_eq!(manifest.groups.len(), reloaded.groups.len());
        for (key, ws) in &manifest.workspaces {
            let reloaded_ws = reloaded.workspaces.get(key).unwrap();
            assert_eq!(ws.version, reloaded_ws.version);
            assert_eq!(ws.path, reloaded_ws.path);
        }
        for (key, children) in &manifest.groups {
            assert_eq!(children, reloaded.groups.get(key).unwrap());
        }
    }

    #[test]
    fn nested_workspace_changelogs_dir_uses_leaf_path() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());

        // Workspace mode: changelogs live at {workspace_path}/.boop/changelogs/
        let cl_dir = changelogs_dir(dir.path(), "typescript/core").unwrap();
        assert_eq!(cl_dir, dir.path().join("typescript/core/.boop/changelogs"));
    }

    #[test]
    fn nested_workspace_name_field_preserved() {
        let dir = setup_dir();
        // Create a nested workspace with name field
        fs::create_dir_all(dir.path().join(".boop")).unwrap();
        fs::write(
            dir.path().join(".boop/releases.toml"),
            "workspaces = [\"packages/core\"]\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("packages/core/.boop")).unwrap();
        fs::write(
            dir.path().join("packages/core/.boop/releases.toml"),
            "name = \"@myorg/core\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let manifest = read_manifest(dir.path()).unwrap();
        let ws = manifest.workspaces.get("packages/core").unwrap();
        assert_eq!(ws.name.as_deref(), Some("@myorg/core"));
        assert_eq!(ws.version, "1.0.0");

        // Round-trip preserves name
        write_manifest(dir.path(), &manifest).unwrap();
        let reloaded = read_manifest(dir.path()).unwrap();
        assert_eq!(
            reloaded
                .workspaces
                .get("packages/core")
                .unwrap()
                .name
                .as_deref(),
            Some("@myorg/core")
        );
    }

    #[test]
    fn nested_workspace_is_workspace_mode() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());

        assert!(is_workspace_mode(dir.path()).unwrap());
    }

    #[test]
    fn nested_workspace_write_updates_leaf_version() {
        let dir = setup_dir();
        setup_nested_workspace_tree(dir.path());

        let mut manifest = read_manifest(dir.path()).unwrap();
        manifest
            .workspaces
            .get_mut("typescript/core")
            .unwrap()
            .version = "1.3.0".to_string();

        write_manifest(dir.path(), &manifest).unwrap();

        // Verify leaf manifest on disk was updated
        let leaf_content =
            fs::read_to_string(dir.path().join("typescript/core/.boop/releases.toml")).unwrap();
        assert!(leaf_content.contains("version = \"1.3.0\""));

        // Verify round-trip
        let reloaded = read_manifest(dir.path()).unwrap();
        assert_eq!(
            reloaded.workspaces.get("typescript/core").unwrap().version,
            "1.3.0"
        );
    }
}
