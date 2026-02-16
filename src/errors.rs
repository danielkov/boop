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
    #[error(transparent)]
    Io(#[from] std::io::Error),
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
}

#[derive(Debug, thiserror::Error)]
pub enum VersionError {
    #[error("invalid version: {input}")]
    InvalidVersion { input: String },
}

#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
