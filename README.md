# boop

Semver release management for any project. No plugins, no config files, no ecosystem lock-in.

```sh
boop init                          # detects version from your manifest
boop minor "Added CSV export"      # record a change
boop apply                         # bump version, cut a release
```

boop stores changelog fragments as individual files, so parallel branches never conflict. At release time, fragments are assembled into a changelog and the version is resolved from the highest pending bump.

## Install

**macOS / Linux:**

```sh
curl -sSfL https://raw.githubusercontent.com/danielkov/boop/main/scripts/install.sh | sh
```

**Windows:**

```powershell
irm https://raw.githubusercontent.com/danielkov/boop/main/scripts/install.ps1 | iex
```

**Cargo:**

```sh
cargo install --git https://github.com/danielkov/boop
```

## Workflow

```sh
# record changes as you work
boop major "Redesigned authentication API"
boop minor "Added dark mode support"
boop patch "Fixed off-by-one in pagination"

# check what's pending
boop status

# cut the release (version resolved from highest bump)
boop apply          # "Version updated to 2.0.0"

# query changelogs
boop version                    # 2.0.0
boop changelog                  # current version
boop changelog 1.0.0            # specific version
boop changelog 1.0.0...2.0.0   # range

# pre-release workflow
boop apply --pre beta           # 2.1.0-beta.0
boop apply --pre beta           # 2.1.0-beta.1
boop apply                      # 2.1.0
```

## CI automation

boop is designed to run in CI. A typical GitHub Actions workflow:

```yaml
- name: Apply pending changes
  id: apply
  run: |
    if boop apply; then
      echo "released=true" >> "$GITHUB_OUTPUT"
    fi

- name: Release
  if: steps.apply.outputs.released == 'true'
  run: |
    VERSION=$(boop version)
    # update your manifest, commit, tag, create GH release
    gh release create "v${VERSION}" --notes "$(boop changelog)"
```

See [boop's own release workflow](.github/workflows/release.yml) for a complete example with cross-platform binary builds.

## Version auto-detection

`boop init` reads the current version from whichever manifest it finds first:

Cargo.toml, package.json, pyproject.toml, setup.cfg, go.mod (via git tags), pom.xml, build.gradle(.kts), _.gemspec, mix.exs, pubspec.yaml, composer.json, _.csproj, CMakeLists.txt, deno.json(c)

No manifest? Starts at `0.0.1`.

## How it works

```
.boop/
  releases.toml          # version + release ledger
  changelogs/
    major-01HQ1X....md   # one file per change, ULID-named
    minor-01HQ2Y....md
    patch-01HQ3Z....md
```

- `boop major|minor|patch` creates a markdown file in `.boop/changelogs/`
- `boop apply` finds the highest pending bump, resolves the next semver version, records the release in `releases.toml`
- `boop changelog` assembles entries grouped by severity (major > minor > patch)
- Everything is plain files — commit `.boop/` to your repo
