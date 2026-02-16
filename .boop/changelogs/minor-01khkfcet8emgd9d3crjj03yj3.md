## GitHub Action for CI/CD integration

Boop now ships a reusable GitHub Action that handles installation, version resolution, and changelog extraction in a single step.

### Setup

```yaml
- uses: danielkov/boop@v1
  id: boop
  with:
    boop-cli-version: 'latest' # optional, pin to a specific version if needed

- if: steps.boop.outputs.released == 'true'
  run: |
    echo "Released ${{ steps.boop.outputs.version }}"
    echo "${{ steps.boop.outputs.changelog }}"
```

### What it does

1. Installs the boop CLI from GitHub Releases (supports Linux, macOS, and Windows runners)
2. Runs `boop apply` to resolve and apply any pending version bump
3. Exposes three step outputs:
   - **`released`** — `true` if there were pending changes, `false` otherwise
   - **`version`** — the newly applied version (e.g., `2.1.0`)
   - **`changelog`** — the assembled changelog markdown for the release

### Other changes

- The boop release workflow now uses its own action (`uses: ./`) instead of building the CLI from source
- Release tags now include a floating major version tag (e.g., `v2`) that tracks the latest stable release, enabling `@v1`-style action references