use std::collections::BTreeMap;
use std::path::Path;

use crate::detect;
use crate::errors::InitError;
use crate::store::{self, Manifest};

pub fn run(base: &Path, version: Option<&str>) -> Result<(), InitError> {
    if store::is_initialized(base) {
        return Err(InitError::AlreadyInitialized {
            path: store::boop_dir(base),
        });
    }

    let version = match version {
        Some(v) => {
            semver::Version::parse(v).map_err(|_| InitError::InvalidVersion {
                input: v.to_string(),
            })?;
            v.to_string()
        }
        None => match detect::detect_version(base) {
            Ok(Some(v)) => v,
            _ => "0.0.1".to_string(),
        },
    };

    store::create_boop_dir(base)?;

    let manifest = Manifest {
        version,
        releases: BTreeMap::new(),
    };
    store::write_manifest(base, &manifest)?;

    Ok(())
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
        run(dir.path(), None).unwrap();
        assert!(dir.path().join(".boop").exists());
        assert!(dir.path().join(".boop/changelogs").exists());
    }

    #[test]
    fn creates_releases_toml_with_default_version() {
        let dir = setup_dir();
        run(dir.path(), None).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.version, "0.0.1");
        assert!(manifest.releases.is_empty());
    }

    #[test]
    fn creates_releases_toml_with_explicit_version() {
        let dir = setup_dir();
        run(dir.path(), Some("1.2.3")).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.version, "1.2.3");
    }

    #[test]
    fn errors_if_already_initialized() {
        let dir = setup_dir();
        run(dir.path(), None).unwrap();
        let err = run(dir.path(), None).unwrap_err();
        assert!(matches!(err, InitError::AlreadyInitialized { .. }));
    }

    #[test]
    fn errors_on_invalid_semver() {
        let dir = setup_dir();
        let err = run(dir.path(), Some("not-a-version")).unwrap_err();
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
        run(dir.path(), None).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.version, "3.0.0");
    }

    #[test]
    fn explicit_version_overrides_heuristic() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"3.0.0\"\n",
        )
        .unwrap();
        run(dir.path(), Some("5.0.0")).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.version, "5.0.0");
    }

    #[test]
    fn falls_back_to_default_when_no_heuristic_match() {
        let dir = setup_dir();
        // No package manifest files → should fall back to 0.0.1
        run(dir.path(), None).unwrap();
        let manifest = store::read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.version, "0.0.1");
    }
}
