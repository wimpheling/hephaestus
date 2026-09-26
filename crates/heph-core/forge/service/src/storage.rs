use forge_domain::RepositoryId;
use std::path::{Path, PathBuf};

mod error;
mod read;
mod repository;

#[cfg(test)]
mod tests;

pub use error::GitStorageError;

const GIT_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Canonical storage manager for bare Git repositories.
#[derive(Debug, Clone)]
pub struct GitStorage {
    root: PathBuf,
}

impl GitStorage {
    /// Creates and canonicalizes the configured repository root.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be created or canonicalized.
    pub async fn initialize(root: impl AsRef<Path>) -> Result<Self, GitStorageError> {
        tokio::fs::create_dir_all(root.as_ref())
            .await
            .map_err(GitStorageError::Io)?;
        let root = tokio::fs::canonicalize(root.as_ref())
            .await
            .map_err(GitStorageError::Io)?;
        if !root.is_dir() {
            return Err(GitStorageError::InvalidRoot(root));
        }
        Ok(Self { root })
    }

    /// Returns the canonical repository root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a route component into an opaque repository identifier.
    ///
    /// Only canonical hyphenated UUID text is accepted. Names, slashes,
    /// percent-decoded traversal, and alternate UUID encodings are rejected.
    ///
    /// # Errors
    ///
    /// Returns an error when `component` is not canonical opaque-ID text.
    pub fn parse_route(component: &str) -> Result<RepositoryId, GitStorageError> {
        let id = component
            .parse::<RepositoryId>()
            .map_err(|_| GitStorageError::InvalidRepositoryRoute(component.to_owned()))?;
        if component != id.to_string() {
            return Err(GitStorageError::InvalidRepositoryRoute(
                component.to_owned(),
            ));
        }
        Ok(id)
    }

    /// Derives the one permitted on-disk location for a repository.
    #[must_use]
    pub fn repository_path(&self, repository_id: RepositoryId) -> PathBuf {
        self.root.join(format!("{repository_id}.git"))
    }
}
