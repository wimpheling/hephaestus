//! Bare repository lifecycle operations.

use super::{GitStorage, GitStorageError};
use std::{path::PathBuf, process::Stdio};
use tokio::process::Command;

impl GitStorage {
    /// Initializes a bare repository in its canonical location.
    ///
    /// # Errors
    ///
    /// Returns an error if the location exists or `git init --bare` fails.
    pub async fn create_bare(
        &self,
        repository_id: forge_domain::RepositoryId,
        default_branch: &str,
    ) -> Result<PathBuf, GitStorageError> {
        let path = self.repository_path(repository_id);
        if tokio::fs::try_exists(&path)
            .await
            .map_err(GitStorageError::Io)?
        {
            return Err(GitStorageError::AlreadyExists(repository_id));
        }
        let output = Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg(format!("--initial-branch={default_branch}"))
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(GitStorageError::Io)?;
        if !output.status.success() {
            return Err(GitStorageError::Git(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        self.validate_existing(repository_id).await
    }

    /// Resolves and verifies an existing repository without following a
    /// repository-level symlink outside the canonical layout.
    ///
    /// # Errors
    ///
    /// Returns an error for missing repositories, symlinks, or non-bare
    /// directories.
    pub async fn validate_existing(
        &self,
        repository_id: forge_domain::RepositoryId,
    ) -> Result<PathBuf, GitStorageError> {
        let expected = self.repository_path(repository_id);
        let metadata = tokio::fs::symlink_metadata(&expected)
            .await
            .map_err(GitStorageError::Io)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(GitStorageError::NonCanonical(expected));
        }
        let actual = tokio::fs::canonicalize(&expected)
            .await
            .map_err(GitStorageError::Io)?;
        if actual != expected {
            return Err(GitStorageError::NonCanonical(actual));
        }
        if !tokio::fs::try_exists(expected.join("HEAD"))
            .await
            .map_err(GitStorageError::Io)?
        {
            return Err(GitStorageError::NotBare(repository_id));
        }
        Ok(expected)
    }

    /// Removes one validated canonical bare repository.
    ///
    /// # Errors
    ///
    /// Returns an error unless the target is the expected non-symlink bare
    /// repository derived from `repository_id`.
    pub async fn delete_bare(
        &self,
        repository_id: forge_domain::RepositoryId,
    ) -> Result<(), GitStorageError> {
        let path = self.validate_existing(repository_id).await?;
        tokio::fs::remove_dir_all(path)
            .await
            .map_err(GitStorageError::Io)
    }
}
