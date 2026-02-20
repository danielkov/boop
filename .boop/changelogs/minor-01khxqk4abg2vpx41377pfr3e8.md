## Rework workspace init with named groups and explicit creation

The `boop init` CLI now supports fully explicit workspace tree construction with named groups.

### New CLI

```
boop init [dir] [-w] [-n <name>] [--version <version>] [--default]
```

- `boop init` — single leaf at `.` (legacy, unchanged)
- `boop init -w` — workspace root group at `.`
- `boop init -w <dir>` — create a named group at `<dir>`
- `boop init <dir>` — create a leaf workspace under the closest parent group

### Named groups and workspaces

The `-n`/`--name` flag sets a custom name (used as the workspace/group key) independent of the filesystem path. Names default to the relative path from the closest parent group.

```sh
boop init -w
boop init -w changelogs/typescript -n typescript
boop init changelogs/typescript/core -n core
boop major -w core "breaking change"
```

### Data model: `GroupInfo`

Groups are now stored as `GroupInfo { path, children }` instead of plain `Vec<String>`, decoupling the group name (map key) from the filesystem path. Group manifests write a `name` field to disk so names round-trip correctly.

### Longest-prefix workspace resolution

`-w` selectors now use longest-prefix matching: `boop major -w typescript/core` first tries exact match, then finds group `typescript` and resolves `core` as a child. This works regardless of whether names contain `/`.

### Other changes

- Empty workspace groups are now allowed (a just-created group with no children)
- Name collision detection prevents duplicate group/workspace names
- All commands (`add`, `apply`, `status`, `changelog`, `version`) work with the new named workspace model
