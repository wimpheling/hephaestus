use forge_domain::RepositoryId;
use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::process::Command;

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

    /// Initializes a bare repository in its canonical location.
    ///
    /// # Errors
    ///
    /// Returns an error if the location exists or `git init --bare` fails.
    pub async fn create_bare(
        &self,
        repository_id: RepositoryId,
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
        repository_id: RepositoryId,
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
}

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
}

#[cfg(test)]
mod tests {
    use super::GitStorage;
    use forge_domain::RepositoryId;

    #[tokio::test]
    async fn derives_only_canonical_opaque_paths() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let storage = GitStorage::initialize(temporary.path())
            .await
            .expect("storage");
        let id = RepositoryId::new();
        assert_eq!(
            storage.repository_path(id),
            temporary
                .path()
                .canonicalize()
                .expect("canonical")
                .join(format!("{id}.git"))
        );
        assert_eq!(GitStorage::parse_route(&id.to_string()).expect("id"), id);
        assert!(GitStorage::parse_route("../etc").is_err());
        assert!(GitStorage::parse_route(&format!("{id}/../../etc")).is_err());
        assert!(GitStorage::parse_route(&id.to_string().replace('-', "")).is_err());
    }

    #[tokio::test]
    async fn creates_and_validates_bare_repository() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let storage = GitStorage::initialize(temporary.path())
            .await
            .expect("storage");
        let id = RepositoryId::new();
        let path = storage
            .create_bare(id, "main")
            .await
            .expect("bare repository");
        assert_eq!(storage.validate_existing(id).await.expect("existing"), path);
    }
}
