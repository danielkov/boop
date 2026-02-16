// shared changelog assembly logic

use std::path::Path;

use crate::errors::StoreError;
use crate::store;
use crate::version::{BumpKind, parse_kind};

pub fn assemble_changelog(
    base: &Path,
    version: &str,
    entry_filenames: &[String],
) -> Result<String, StoreError> {
    // Group entries by kind, collecting (filename, content) pairs
    let mut major_entries: Vec<(&str, String)> = Vec::new();
    let mut minor_entries: Vec<(&str, String)> = Vec::new();
    let mut patch_entries: Vec<(&str, String)> = Vec::new();

    for filename in entry_filenames {
        let content = store::read_entry(base, filename)?;
        match parse_kind(filename) {
            Some(BumpKind::Major) => major_entries.push((filename, content)),
            Some(BumpKind::Minor) => minor_entries.push((filename, content)),
            Some(BumpKind::Patch) => patch_entries.push((filename, content)),
            None => {
                // Skip entries with unrecognized kind prefix
            }
        }
    }

    // Sort within each group by filename (ULID portion ensures chronological order)
    major_entries.sort_by_key(|(f, _)| *f);
    minor_entries.sort_by_key(|(f, _)| *f);
    patch_entries.sort_by_key(|(f, _)| *f);

    // Concatenate: major first, then minor, then patch
    let all_entries: Vec<&str> = major_entries
        .iter()
        .chain(minor_entries.iter())
        .chain(patch_entries.iter())
        .map(|(_, content)| content.as_str())
        .collect();

    let mut output = format!("# {version}\n");

    for entry_content in &all_entries {
        output.push('\n');
        output.push_str(entry_content.trim_end());
        output.push('\n');
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_test_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let changelogs = dir.path().join(".boop/changelogs");
        fs::create_dir_all(&changelogs).unwrap();
        dir
    }

    fn write_entry(dir: &Path, filename: &str, content: &str) {
        let path = dir.join(".boop/changelogs").join(filename);
        fs::write(path, content).unwrap();
    }

    #[test]
    fn basic_assembly() {
        let dir = setup_test_dir();
        write_entry(
            dir.path(),
            "minor-01HQ1XABC.md",
            "## Added CSV export\n\nUsers can now export reports as CSV.",
        );
        write_entry(
            dir.path(),
            "patch-01HQ2YDEF.md",
            "## Fixed login bug\n\nThe login form no longer crashes on empty input.",
        );

        let filenames = vec![
            "minor-01HQ1XABC.md".to_string(),
            "patch-01HQ2YDEF.md".to_string(),
        ];

        let result = assemble_changelog(dir.path(), "1.3.0", &filenames).unwrap();

        let expected = "\
# 1.3.0

## Added CSV export

Users can now export reports as CSV.

## Fixed login bug

The login form no longer crashes on empty input.
";
        assert_eq!(result, expected);
    }

    #[test]
    fn groups_sorted_major_minor_patch() {
        let dir = setup_test_dir();
        write_entry(dir.path(), "patch-01HQ1.md", "## Patch fix");
        write_entry(dir.path(), "major-01HQ2.md", "## Breaking change");
        write_entry(dir.path(), "minor-01HQ3.md", "## New feature");

        let filenames = vec![
            "patch-01HQ1.md".to_string(),
            "major-01HQ2.md".to_string(),
            "minor-01HQ3.md".to_string(),
        ];

        let result = assemble_changelog(dir.path(), "2.0.0", &filenames).unwrap();

        // Major should come first, then minor, then patch
        let major_pos = result.find("## Breaking change").unwrap();
        let minor_pos = result.find("## New feature").unwrap();
        let patch_pos = result.find("## Patch fix").unwrap();
        assert!(major_pos < minor_pos);
        assert!(minor_pos < patch_pos);
    }

    #[test]
    fn within_group_sorted_by_ulid() {
        let dir = setup_test_dir();
        write_entry(dir.path(), "minor-01HQ3ZZZ.md", "## Later feature");
        write_entry(dir.path(), "minor-01HQ1AAA.md", "## Earlier feature");

        let filenames = vec![
            "minor-01HQ3ZZZ.md".to_string(),
            "minor-01HQ1AAA.md".to_string(),
        ];

        let result = assemble_changelog(dir.path(), "1.1.0", &filenames).unwrap();

        let earlier_pos = result.find("## Earlier feature").unwrap();
        let later_pos = result.find("## Later feature").unwrap();
        assert!(earlier_pos < later_pos);
    }

    #[test]
    fn empty_entries() {
        let _dir = setup_test_dir();
        let filenames: Vec<String> = vec![];

        let result = assemble_changelog(_dir.path(), "1.0.0", &filenames).unwrap();
        assert_eq!(result, "# 1.0.0\n");
    }

    #[test]
    fn skips_unknown_kind() {
        let dir = setup_test_dir();
        write_entry(dir.path(), "minor-01HQ1.md", "## Feature");
        write_entry(dir.path(), "unknown-01HQ2.md", "## Mystery");

        let filenames = vec!["minor-01HQ1.md".to_string(), "unknown-01HQ2.md".to_string()];

        let result = assemble_changelog(dir.path(), "1.1.0", &filenames).unwrap();

        assert!(result.contains("## Feature"));
        assert!(!result.contains("## Mystery"));
    }
}
