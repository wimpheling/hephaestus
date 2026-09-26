//! Errors returned by canonical Git repository storage.

use forge_domain::RepositoryId;
use std::{io, path::PathBuf};

/// Bare repository storage failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GitStorageError {
    /// Filesystem operation failed.
    #[error("repository storage I/O failed: {0}")]
    Io(#[source] io::Error),
    /// Configured root is not a directory.
    #[error("repository root is not a directory: {0}")]
    InvalidRoot(PathBuf),
    /// Route text is not one canonical opaque identifier.
    #[error("invalid opaque repository route {0:?}")]
    InvalidRepositoryRoute(String),
    /// Repository already exists.
    #[error("repository {0} already exists")]
    AlreadyExists(RepositoryId),
    /// Repository resolves outside its canonical location.
    #[error("repository path is not canonical: {0}")]
    NonCanonical(PathBuf),
    /// Repository does not look like a bare Git repository.
    #[error("repository {0} is not a bare Git repository")]
    NotBare(RepositoryId),
    /// Git command failed.
    #[error("git repository operation failed: {0}")]
    Git(String),
    /// Commit text is not a full lowercase hexadecimal object name.
    #[error("invalid Git commit object name")]
    InvalidCommit,
    /// Path is not a safe repository-relative path.
    #[error("invalid repository-relative path")]
    InvalidPath,
    /// Git output exceeded the requested bound.
    #[error("Git output exceeded its bound")]
    OutputTooLarge,
}
