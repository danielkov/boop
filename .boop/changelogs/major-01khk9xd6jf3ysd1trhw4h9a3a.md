## boop v1 — Language-agnostic release management

A CLI tool that manages semver versioning and changelogs for any project, regardless of language or build system.

### Capabilities

- **Auto-detection** of current version from 14+ manifest formats (Cargo.toml, package.json, pyproject.toml, go.mod via git tags, pom.xml, build.gradle, gemspec, mix.exs, pubspec.yaml, composer.json, .csproj, CMakeLists.txt, deno.json, setup.cfg)
- **Semver resolution** with major/minor/patch bumps and full pre-release workflow (alpha, beta, rc with auto-incrementing counters)
- **Fragment-based changelogs** — each change is an independent markdown file (ULID-named for conflict-free parallel work), assembled at release time grouped by severity
- **Release ledger** (.boop/releases.toml) tracks version history and maps releases to their changelog entries
- **Changelog queries** by single version or semver range
- **Status command** showing current version and pending unreleased entries

### Usage

```sh
# Initialize in any project (auto-detects version from manifest)
boop init

# Record changes as you go
boop major "## Redesigned API surface"
boop minor "## Added CSV export"
boop patch "## Fixed login crash on empty input"

# Check what's pending
boop status

# Cut a release (resolves semver bump from highest pending entry)
boop apply

# Pre-release workflow
boop apply --pre beta   # 1.1.0-beta.0
boop apply --pre beta   # 1.1.0-beta.1
boop apply              # 1.1.0

# Query changelog history
boop changelog              # current version
boop changelog 1.2.0        # specific version
boop changelog 1.0.0...2.0.0  # range
```