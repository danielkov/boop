use std::path::Path;

use crate::errors::BoopError;
use crate::store;

pub fn run(base: &Path) -> Result<(), BoopError> {
    store::ensure_initialized(base)?;
    let manifest = store::read_manifest(base)?;
    println!("{}", manifest.version);
    Ok(())
}
