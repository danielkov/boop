# RFC 002: Workspace-Aware Monorepo Support

## Summary

Add workspace-scoped versioning to `boop` with a single repo-level `.boop/` store.

- Each workspace has independent version + release history.
- Changelog files are partitioned by workspace directory.
- Multi-workspace changelog creation is supported by duplicating files (same ID, different dirs).
- Every apply run writes one `release_group` identifier across all releases produced in that run.
- `boop apply --dry-run` prints planned release changes without filesystem writes.
- `boop revert` rolls back the most recent apply group.
- No synthetic `total` workspace.
- No manifest schema version field; loader should attempt new shape first, then legacy shape.

## Goals

- Independent versioning per workspace.
- Support `boop ... -w a,b,c` without adding cross-workspace global entry registries.
- Keep data model simple and file-centric.
- Preserve existing single-workspace behavior.
- Enable rollout-level correlation for multi-workspace releases.

## Non-Goals

- First-class linked multi-workspace entries in manifest.
- Synthetic aggregate version (`total`).

## Data Model

### Directory Layout

```text
.boop/
  releases.toml
  changelogs/
    root/
      major-<id>.md
      minor-<id>.md
      patch-<id>.md
    api/
      major-<id>.md
    web/
      patch-<id>.md
```

### Manifest Shape

```toml
default_workspace = "root"

[workspaces.root]
path = "."
version = "1.4.0"

[workspaces.api]
path = "apps/api"
version = "2.1.0"

[workspaces.web]
path = "apps/web"
version = "0.8.3"

[workspaces.api.releases."2.1.0"]
entries = ["minor-01abc.md", "patch-01def.md"]
release_group = "rg-01xyz"

[workspaces.web.releases."0.8.3"]
entries = ["major-01abc.md"]
release_group = "rg-01xyz"

[[release_groups]]
id = "rg-01xyz"
workspaces = ["api", "web"]
before = { api = "2.0.4", web = "0.8.2" }
after = { api = "2.1.0", web = "0.8.3" }
```

```toml
[[release_groups]]
id = "rg-01xzz"
workspaces = ["root"]
before = { root = "1.4.0" }
after = { root = "1.4.1" }
```

Notes:

- `entries` stores filenames only (no workspace prefix) because workspace is implied by release table path.
- IDs are filename IDs and may intentionally repeat across workspaces.
- `release_group` is written for every release created by `apply` (single or multi-workspace).
- `release_groups` is append-only and defines global apply order for deterministic `boop revert`.
- `release_groups` is a delta log: each record stores `before` and `after` workspace version maps for that apply run.
- Revert is implemented by popping the last `release_groups` item and undoing only the workspace versions in `after`.

## Command Semantics

### `boop init`

- Initializes `.boop/` with `default_workspace = "root"`.
- Creates `workspaces.root` and `.boop/changelogs/root/`.
- Existing single-project version detection applies to `root`.

### `boop major|minor|patch [-w <csv>] <message>`

- `-w` accepts comma-separated workspace names.
- No `-w` targets `default_workspace`.
- For each target workspace, write one file to `.boop/changelogs/<workspace>/`.
- For multi-workspace invocation, reuse one generated `<id>` across all written files.
  - Example: `major-01xyz.md` appears in `api/`, `web/`, and `sdk/`.

### `boop apply [-w <csv> | --all] [--pre [<tag>]] [--dry-run]`

- `-w` accepts comma-separated workspace names.
- `-w` with one workspace applies pending entries for that workspace.
- `-w` with multiple workspaces applies pending entries for that selected set.
- `--all` applies each workspace with pending entries.
- No selector: apply `default_workspace`.
- Version resolution is unchanged but computed per workspace using only that workspace pending entries.
- On every successful non-dry-run apply, generate one `release_group` ID, attach it to each workspace release created in that run, and append one `release_groups` delta record with `before` + `after` maps.
- `--dry-run` prints the same computed plan (workspace, current version, next version, selected entries, release_group) but performs no writes.

### `boop revert`

- Reverts the most recent apply group by popping the last item in `release_groups`.
- Default scope is global release-group order, not per-workspace order.
- For each workspace in that release group:
  - Remove the released version entry from `workspaces.<name>.releases` using `release_groups.after.<workspace>`.
  - Reset `workspaces.<name>.version` using `release_groups.before.<workspace>`.
- Remove the reverted group record from `release_groups`.
- Changelog files are not deleted; reverted entries become pending again via normal pending computation.
- Errors if no prior apply operation exists.

### `boop version [-w <workspace>]`

- Returns version for the selected workspace.
- In monorepo mode, no `-w` resolves to `default_workspace`.

### `boop changelog [-w <csv>] [<range>]`

- `-w` accepts one or many workspaces.
- Single workspace behaves like current changelog query semantics (current/single/range).
- Multiple workspaces print each workspace section independently.
- Correlation across workspaces can be inferred by shared filename ID.
- Optional filter: `--group <release_group>` prints releases correlated by that rollout ID.

## Pending Entry Computation

For workspace `X`:

1. List files in `.boop/changelogs/X/`.
2. Collect referenced filenames from `workspaces.X.releases.*.entries`.
3. Pending = filesystem filenames minus referenced filenames.

No cross-workspace pending tracking.

## Backward Compatibility

- Loader should attempt workspace-aware manifest first.
- If parse fails, attempt legacy manifest (`version` + `releases` at top level).
- Legacy manifests map to:
  - `default_workspace = "root"`
  - `workspaces.root.version = <legacy version>`
  - `workspaces.root.releases = <legacy releases>`
  - changelog files treated as `root` workspace inputs.

## Tradeoffs

- Multi-workspace changelog creation duplicates files by design.
- This avoids global entry registry complexity while preserving optional correlation through shared IDs.
- `release_group` adds minimal orchestration metadata while keeping entries file-centric.
