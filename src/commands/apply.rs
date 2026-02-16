// boop apply — version resolution + changelog assembly

use std::path::Path;

use crate::errors::ApplyError;
use crate::store::{self, Release};
use crate::version;

pub fn run(base: &Path, pre_tag: Option<Option<&str>>) -> Result<(), ApplyError> {
    store::ensure_initialized(base)?;

    let mut manifest = store::read_manifest(base)?;
    let all_entries = store::list_entry_filenames(base)?;
    let pending = store::pending_entries(&manifest, &all_entries);

    if pending.is_empty() {
        return Err(ApplyError::NoPendingEntries);
    }

    let bump = version::highest_bump(&pending).unwrap();

    let current = semver::Version::parse(&manifest.version).map_err(|_| {
        crate::errors::VersionError::InvalidVersion {
            input: manifest.version.clone(),
        }
    })?;

    let next_version = version::resolve_next_version(&current, bump, pre_tag)?;

    println!("Version updated to {next_version}");

    manifest
        .releases
        .insert(next_version.to_string(), Release { entries: pending });
    manifest.version = next_version.to_string();

    store::write_manifest(base, &manifest)?;

    Ok(())
}
