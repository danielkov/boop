use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum BoopError {
    #[error(transparent)]
    Init(#[from] InitError),
    #[error(transparent)]
    Add(#[from] AddError),
    #[error(transparent)]
    Apply(#[from] ApplyError),
    #[error(transparent)]
    Changelog(#[from] ChangelogError),
    #[error(transparent)]
    Status(#[from] StatusError),
    #[error(transparent)]
    Revert(#[from] RevertError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Version(#[from] VersionError),
    #[error(transparent)]
    Detect(#[from] DetectError),
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error(".boop/ already exists at {path}")]
    AlreadyInitialized { path: PathBuf },
    #[error(".boop/ not found — run `boop init` first")]
    NotInitialized,
    #[error("cannot add workspace under {path}: it is a leaf workspace, not a group")]
    ParentIsLeaf { path: String },
    #[error("--name requires -w")]
    NameWithoutWorkspace,
    #[error("invalid version: {input}")]
    InvalidVersion { input: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, thiserror::Error)]
pub enum AddError {
    #[error(".boop/ not found — run `boop init` first")]
    NotInitialized,
    #[error("unknown workspace: {name}")]
    UnknownWorkspace { name: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error(".boop/ not found — run `boop init` first")]
    NotInitialized,
    #[error("no pending changelog entries to apply")]
    NoPendingEntries,
    #[error("unknown workspace: {name}")]
    UnknownWorkspace { name: String },
    #[error("in workspace mode, choose targets with -w <csv> or --all")]
    WorkspaceSelectorRequired,
    #[error("cannot use --current with --pre")]
    ConflictingCurrentAndPre,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Version(#[from] VersionError),
}

#[derive(Debug, thiserror::Error)]
pub enum ChangelogError {
    #[error(".boop/ not found — run `boop init` first")]
    NotInitialized,
    #[error("workspace not found: {name}")]
    WorkspaceNotFound { name: String },
    #[error("release group not found: {id}")]
    ReleaseGroupNotFound { id: String },
    #[error("version {version} not found in releases")]
    VersionNotFound { version: String },
    #[error("invalid range: {input}")]
    InvalidRange { input: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, thiserror::Error)]
pub enum StatusError {
    #[error(".boop/ not found — run `boop init` first")]
    NotInitialized,
    #[error("unknown workspace: {name}")]
    UnknownWorkspace { name: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, thiserror::Error)]
pub enum RevertError {
    #[error(".boop/ not found — run `boop init` first")]
    NotInitialized,
    #[error("no prior apply operation to revert")]
    NothingToRevert,
    #[error("cannot determine prior version to revert to")]
    NoPriorVersion,
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(".boop/ not found — run `boop init` first")]
    NotInitialized,
    #[error("corrupt manifest: {reason}")]
    CorruptManifest { reason: String },
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("failed to serialize releases.toml: {source}")]
    Serialize { source: toml::ser::Error },
    #[error("invalid workspace name/path {name:?}: use relative paths like `apps/api` or `.`")]
    InvalidWorkspaceName { name: String },
    #[error("unknown workspace: {name}")]
    UnknownWorkspace { name: String },
    #[error("{name:?} is a workspace group — use --all to target all workspaces within it")]
    WorkspaceIsGroup { name: String },
}

#[derive(Debug, thiserror::Error)]
pub enum VersionError {
    #[error("invalid version: {input}")]
    InvalidVersion { input: String },
    #[error("unknown workspace: {name}")]
    UnknownWorkspace { name: String },
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
