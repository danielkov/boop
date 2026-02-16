// .boop/ filesystem operations, manifest read/write

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::StoreError;

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub releases: BTreeMap<String, Release>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Release {
    pub entries: Vec<String>,
}

pub fn boop_dir(base: &Path) -> PathBuf {
    base.join(".boop")
}

pub fn changelogs_dir(base: &Path) -> PathBuf {
    base.join(".boop/changelogs")
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
    let changelogs = changelogs_dir(base);
    fs::create_dir_all(&changelogs).map_err(|source| StoreError::Write {
        path: changelogs,
        source,
    })
}

pub fn read_manifest(base: &Path) -> Result<Manifest, StoreError> {
    let path = manifest_path(base);
    let content = fs::read_to_string(&path).map_err(|source| StoreError::Read {
        path: path.clone(),
        source,
    })?;
    let manifest: Manifest =
        toml::from_str(&content).map_err(|source| StoreError::Parse { path, source })?;
    Ok(manifest)
}

pub fn write_manifest(base: &Path, manifest: &Manifest) -> Result<(), StoreError> {
    let path = manifest_path(base);
    let content =
        toml::to_string_pretty(manifest).map_err(|source| StoreError::Serialize { source })?;
    fs::write(&path, content).map_err(|source| StoreError::Write { path, source })
}

pub fn write_entry(base: &Path, filename: &str, content: &str) -> Result<PathBuf, StoreError> {
    let dir = changelogs_dir(base);
    let path = dir.join(filename);
    fs::write(&path, content).map_err(|source| StoreError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn read_entry(base: &Path, filename: &str) -> Result<String, StoreError> {
    let path = changelogs_dir(base).join(filename);
    fs::read_to_string(&path).map_err(|source| StoreError::Read { path, source })
}

pub fn list_entry_filenames(base: &Path) -> Result<Vec<String>, StoreError> {
    let dir = changelogs_dir(base);
    let entries = fs::read_dir(&dir).map_err(|source| StoreError::Read { path: dir, source })?;

    let mut filenames = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| StoreError::Read {
            path: changelogs_dir(base),
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

pub fn pending_entries(manifest: &Manifest, all_entries: &[String]) -> Vec<String> {
    let referenced: std::collections::HashSet<&str> = manifest
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
