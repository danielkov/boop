use std::path::Path;

use crate::errors::BoopError;
use crate::store;

pub fn run(base: &Path) -> Result<(), BoopError> {
    store::ensure_initialized(base)?;

    let manifest = store::read_manifest(base)?;
    let all_entries = store::list_entry_filenames(base)?;
    let pending = store::pending_entries(&manifest, &all_entries);

    println!("Current version: {}", manifest.version);

    if pending.is_empty() {
        println!("\nNo pending changelog entries.");
    } else {
        println!("\nPending changelog entries:");
        for filename in &pending {
            let kind = filename.split('-').next().unwrap_or("unknown");
            let content = store::read_entry(base, filename)?;
            let first_line = content
                .lines()
                .next()
                .unwrap_or("")
                .trim_start_matches('#')
                .trim();
            println!("  {:<6} {}", kind, first_line);
        }
    }

    Ok(())
}
