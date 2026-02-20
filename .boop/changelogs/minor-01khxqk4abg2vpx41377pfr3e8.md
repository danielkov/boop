## Add workspace creation to `boop init`

Users can now incrementally build workspace trees without manually creating TOML files and directory structures.

### `boop init -w <path>`

Adds a workspace at the given filesystem path to an already-initialized repo. On first use the root manifest is automatically converted to workspace mode — the existing `root` workspace becomes `.` and groups are created for every intermediate directory.

```sh
boop init                  # initialize root
boop init -w apps/api      # add apps/api workspace
boop init -w apps/web      # add apps/web workspace
boop init -w libs/core     # add libs/core workspace
```

Workspace creation is **idempotent**: running the same `init -w` twice is a silent no-op.

### `--name` flag (named workspaces)

Decouples the workspace key from the filesystem path. When `--name` is set the last path segment in the key is replaced with the given name:

```sh
boop init -w packages/ts-sdk --name typescript
# key = packages/typescript, path = packages/ts-sdk
```

This lets multiple workspaces share a directory prefix while keeping human-friendly keys.

### Manifest-aware changelog resolution

All commands (`add`, `apply`, `status`, `changelog`) now resolve changelog directories through the manifest instead of inferring layout from the workspace key. This ensures named workspaces whose key differs from their path read and write entries in the correct location.