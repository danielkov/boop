# RFC 001: boop CLI

## Summary

`boop` is a language-agnostic release management CLI. It decouples changelog authoring from version bumping by storing individual change entries as discrete files, then resolving the next version and assembling the changelog at apply time.

## Data Model

### Directory Structure

```
.boop/
  releases.toml    # version + release-to-entry mapping
  changelogs/      # all change entries (immutable after creation)
    <kind>-<id>.md
```

### Change Entry

Filename: `<kind>-<ulid>.md` where kind is `major`, `minor`, or `patch`.

Content: freeform markdown supplied by the user at creation time. Files are never moved, deleted, or modified after creation.

### Releases Manifest (`releases.toml`)

Single source of truth for current version and release history.

```toml
version = "1.3.0"

[releases."1.2.0"]
entries = ["minor-01HQ1XABC.md", "patch-01HQ2YDEF.md"]

[releases."1.3.0"]
entries = ["minor-01HQ3AGHI.md"]
```

- `version`: current version string.
- `releases`: map of version → list of entry filenames belonging to that release.
- **Pending entries**: files in `changelogs/` whose filename does not appear in any release's `entries` list.

On `boop init`, the manifest is created with the initial version and an empty `[releases]` table.

## Commands

### `boop init`

Initialize `.boop/` directory with `releases.toml` and empty `changelogs/` dir.

Default version: `0.0.1`.

Override: `boop init --version <semver>`.

**Package format heuristic:** On init, scan cwd for known package manifests and extract version if present. Use the discovered version as default instead of `0.0.1`. First match wins, scanned in this order:

| File | Language/Ecosystem | Version extraction |
|---|---|---|
| `Cargo.toml` | Rust | `package.version` |
| `package.json` | Node.js | `.version` |
| `pyproject.toml` | Python | `project.version` or `tool.poetry.version` |
| `setup.cfg` | Python (legacy) | `metadata.version` |
| `go.mod` | Go | parse module version comment or tag convention |
| `pom.xml` | Java/Maven | `<project><version>` |
| `build.gradle` / `build.gradle.kts` | Java/Kotlin/Gradle | `version` property |
| `*.gemspec` | Ruby | `spec.version` |
| `mix.exs` | Elixir | `@version` or `version:` in project/0 |
| `pubspec.yaml` | Dart/Flutter | `version` |
| `composer.json` | PHP | `.version` |
| `*.csproj` | C#/.NET | `<Version>` or `<PackageVersion>` |
| `Package.swift` | Swift | not typically versioned in manifest — skip |
| `CMakeLists.txt` | C/C++ | `project(... VERSION x.y.z)` |
| `deno.json` / `deno.jsonc` | Deno | `.version` |

If `--version` is passed explicitly, it takes precedence over heuristic.

Error if `.boop/` already exists.

### `boop major|minor|patch <message>`

Create a new change entry.

1. Generate a ULID for the filename.
2. Write `<message>` as the file content to `.boop/changelogs/<kind>-<ulid>.md`.

`<message>` is raw markdown. The user is expected to pass a meaningful heading, e.g.:

```
boop minor "## Added CSV export\n\nUsers can now export reports as CSV."
```

Error if `.boop/` does not exist.

### `boop apply [--pre [<tag>]]`

Resolve next version, assemble changelog, and record release.

**Version resolution:**

1. Read current version from `releases.toml`.
2. Compute pending entries: files in `changelogs/` not referenced by any release in the manifest.
3. Determine highest bump kind present (major > minor > patch) from pending entry filenames.
4. Compute next version using semver increment on the determined kind.

**Pre-release handling (`--pre`):**

- `--pre` (flag, no value): use `pre` as the pre-release tag.
- `--pre <tag>`: use `<tag>` as the pre-release tag.

Logic:

| Current | Bump | `--pre` | Result |
|---|---|---|---|
| `1.2.3` | minor | — | `1.3.0` |
| `1.2.3` | minor | `--pre` | `1.3.0-pre.0` |
| `1.2.3` | minor | `--pre beta` | `1.3.0-beta.0` |
| `1.3.0-pre.0` | any | `--pre` | `1.3.0-pre.1` |
| `1.3.0-beta.0` | any | `--pre beta` | `1.3.0-beta.1` |
| `1.3.0-pre.0` | minor | — | `1.3.0` |
| `1.3.0-beta.0` | minor | `--pre rc` | `1.3.0-rc.0` |

When current version is already a pre-release and `--pre` is used with the **same** tag: increment the pre-release numeric component only — do not bump the core version. This allows multiple pre-releases converging on the same target version.

When current version is a pre-release and `--pre` is used with a **different** tag: reset numeric to 0 with new tag, keep core version.

When current version is a pre-release and `--pre` is **not** passed: strip pre-release suffix, release the core version as stable.

**Changelog assembly:**

Shared function used by both `apply` and `changelog` commands:

1. Given a set of entry filenames, read their contents from `changelogs/`.
2. Group entries by kind (parsed from filename prefix).
3. Sort groups: major, minor, patch.
4. Within each group, sort by ULID (chronological).
5. Concatenate all entry contents under a version heading: `# <version>`.

**Apply-specific side effects:**

1. Assemble changelog from pending entries (print to stdout).
2. Add new release entry to `releases.toml` mapping the new version to the pending entry filenames.
3. Update `version` field in `releases.toml`.

Changelog files are **not** moved or deleted.

Error if no pending changelogs exist. Error if `.boop/` does not exist.

### `boop changelog [<range>]`

Query changelogs. Uses the same assembly function as `apply`.

**No argument:** Assemble and print the latest version's changelog by looking up its entries in the manifest.

**Single version** (`boop changelog 1.2.3`): Look up entries for that version in the manifest, assemble, print.

**Range** (`boop changelog 1.2.3...2.0.0`): Collect all versions in the range (inclusive, ordered ascending by semver), assemble each, concatenate, print.

All lookups go through `releases.toml` → entry filenames → `changelogs/` files. Same read path as `apply`.

Error if version(s) not found in manifest.

### `boop status`

Print current version and list pending changelog entries (kind + first line of content) for quick overview. Pending = entries in `changelogs/` not referenced by any release in the manifest.

## Implementation

### Language & Dependencies

- Rust, built with Cargo.
- `clap` for CLI argument parsing (derive API).
- `semver` crate for version parsing and manipulation.
- `ulid` crate for ID generation.
- No async runtime needed — all operations are synchronous filesystem I/O.

### Crate Structure

```
src/
  main.rs          # CLI entrypoint, clap definitions
  lib.rs           # public API re-exports
  commands/
    mod.rs
    init.rs
    add.rs         # major/minor/patch entry creation
    apply.rs       # version resolution + changelog assembly
    changelog.rs   # history query
    status.rs
  version.rs       # semver resolution logic, pre-release handling
  store.rs         # .boop/ filesystem operations, manifest read/write
  assembly.rs      # shared changelog assembly logic
  detect.rs        # package manifest version detection
```

### Error Handling

Use `thiserror` with typed error enums per module. A top-level `BoopError` enum wraps module-specific variants. Each variant carries structured context (e.g. path, version string) — no stringly-typed errors. `Display` impls on error types produce the user-facing messages. Exit code 1 on any error.

```rust
#[derive(Debug, thiserror::Error)]
pub enum BoopError {
    #[error(transparent)]
    Init(#[from] InitError),
    #[error(transparent)]
    Apply(#[from] ApplyError),
    #[error(transparent)]
    Changelog(#[from] ChangelogError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Version(#[from] VersionError),
}
```

Module errors follow the same pattern, e.g.:

```rust
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error(".boop/ already exists at {path}")]
    AlreadyInitialized { path: PathBuf },
    #[error("invalid version: {input}")]
    InvalidVersion { input: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

### Output

- `boop apply` prints the assembled changelog to stdout. Side effects (file writes) are silent unless `--verbose` is passed.
- `boop changelog` prints to stdout.
- `boop status` prints to stdout.
- `boop major|minor|patch` prints the created entry path to stderr as confirmation.

## Edge Cases

- Running `boop apply` with changelogs of only one kind (e.g. only patches) bumps only that component.
- `boop apply` with mixed kinds uses the highest: if both `minor` and `patch` exist, result is a minor bump.
- No pending entries on `boop apply` → error, nothing to apply.
- Version `0.x.y` follows the same rules — no special `0.x` semver handling.
- ULID ensures no filename collisions even under rapid successive calls.
- `boop init` in a repo that already has `.boop/` → error, not overwrite.
