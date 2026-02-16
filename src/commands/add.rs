// boop major/minor/patch entry creation

use std::path::Path;

use crate::errors::AddError;
use crate::store;
use crate::version::BumpKind;

pub fn run(base: &Path, kind: BumpKind, message: &str) -> Result<(), AddError> {
    store::ensure_initialized(base)?;

    let ulid = ulid::Ulid::new().to_string().to_lowercase();
    let filename = format!("{kind}-{ulid}.md");
    let path = store::write_entry(base, &filename, message)?;

    eprintln!("Created {}", path.display());
    Ok(())
}
