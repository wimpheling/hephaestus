use forge_domain::RepositoryId;
use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

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

    /// Reads one bounded blob from an exact commit in a canonical repository.
    ///
    /// The commit and path are passed as individual Git arguments after
    /// validating their narrow object-name/path contracts. This keeps
    /// repository source retrieval independent from branch movement and makes
    /// callers able to bind a file to an immutable release commit.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository, commit, path, Git object, or
    /// output bound is invalid or unavailable.
    pub async fn read_file_at_commit(
        &self,
        repository_id: RepositoryId,
        commit: &str,
        path: &str,
        maximum_bytes: usize,
    ) -> Result<Vec<u8>, GitStorageError> {
        if !valid_commit(commit) {
            return Err(GitStorageError::InvalidCommit);
        }
        if !valid_relative_path(path) {
            return Err(GitStorageError::InvalidPath);
        }
        let repository = self.validate_existing(repository_id).await?;
        let object = format!("{commit}:{path}");
        let mut child = Command::new("git")
            .arg("--git-dir")
            .arg(repository)
            .args(["cat-file", "blob"])
            .arg(object)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(GitStorageError::Io)?;
        let mut output = Vec::with_capacity(maximum_bytes.min(16 * 1024));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| GitStorageError::Git(String::from("Git output unavailable")))?;
        let read = timeout(
            GIT_COMMAND_TIMEOUT,
            stdout
                .take(u64::try_from(maximum_bytes.saturating_add(1)).unwrap_or(u64::MAX))
                .read_to_end(&mut output),
        )
        .await;
        match read {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(GitStorageError::Io(error));
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(GitStorageError::Git(String::from("Git command timed out")));
            }
        }
        if output.len() > maximum_bytes {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(GitStorageError::OutputTooLarge);
        }
        let status = timeout(GIT_COMMAND_TIMEOUT, child.wait())
            .await
            .map_err(|_| GitStorageError::Git(String::from("Git command timed out")))?
            .map_err(GitStorageError::Io)?;
        if !status.success() {
            return Err(GitStorageError::Git(String::from(
                "Git object is unavailable",
            )));
        }
        Ok(output)
    }

    /// Removes one validated canonical bare repository.
    ///
    /// # Errors
    ///
    /// Returns an error unless the target is the expected non-symlink bare
    /// repository derived from `repository_id`.
    pub async fn delete_bare(&self, repository_id: RepositoryId) -> Result<(), GitStorageError> {
        let path = self.validate_existing(repository_id).await?;
        tokio::fs::remove_dir_all(path)
            .await
            .map_err(GitStorageError::Io)
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

fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        && !value
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
}

#[cfg(test)]
mod tests {
    use super::{GitStorage, valid_commit, valid_relative_path};
    use forge_domain::RepositoryId;
    use std::{fs, process::Command};

    fn git(directory: &std::path::Path, arguments: &[&str]) {
        let status = Command::new("git")
            .args(arguments)
            .current_dir(directory)
            .status()
            .expect("run Git");
        assert!(status.success(), "Git command failed: {arguments:?}");
    }

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

    #[tokio::test]
    async fn reads_a_bounded_blob_from_the_exact_commit() {
        let storage_root = tempfile::tempdir().expect("storage root");
        let worktree = tempfile::tempdir().expect("worktree");
        let storage = GitStorage::initialize(storage_root.path())
            .await
            .expect("storage");
        let repository_id = RepositoryId::new();
        let bare = storage
            .create_bare(repository_id, "main")
            .await
            .expect("bare repository");
        git(worktree.path(), &["init", "--initial-branch=main"]);
        git(
            worktree.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(worktree.path(), &["config", "user.name", "Storage Test"]);
        fs::write(worktree.path().join("heph.gateways.toml"), b"version = 1\n").expect("manifest");
        git(worktree.path(), &["add", "heph.gateways.toml"]);
        git(worktree.path(), &["commit", "-m", "manifest"]);
        let commit = String::from_utf8(
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(worktree.path())
                .output()
                .expect("resolve commit")
                .stdout,
        )
        .expect("commit text")
        .trim()
        .to_owned();
        git(
            worktree.path(),
            &["remote", "add", "origin", bare.to_str().expect("bare path")],
        );
        git(worktree.path(), &["push", "origin", "main"]);
        fs::write(worktree.path().join("heph.gateways.toml"), b"version = 2\n")
            .expect("updated manifest");
        git(worktree.path(), &["add", "heph.gateways.toml"]);
        git(worktree.path(), &["commit", "-m", "updated manifest"]);
        git(worktree.path(), &["push", "origin", "main"]);

        assert_eq!(
            storage
                .read_file_at_commit(repository_id, &commit, "heph.gateways.toml", 64)
                .await
                .expect("read exact blob"),
            b"version = 1\n"
        );
        assert!(
            storage
                .read_file_at_commit(repository_id, &commit, "heph.gateways.toml", 4)
                .await
                .is_err()
        );
    }

    #[test]
    fn accepts_only_full_lowercase_commit_names_and_relative_paths() {
        assert!(valid_commit(&"a".repeat(40)));
        assert!(valid_commit(&"b".repeat(64)));
        assert!(!valid_commit(&"A".repeat(40)));
        assert!(!valid_commit(&"a".repeat(39)));
        assert!(valid_relative_path("heph.gateways.toml"));
        assert!(valid_relative_path("config/heph.gateways.toml"));
        assert!(!valid_relative_path("../heph.gateways.toml"));
        assert!(!valid_relative_path("/heph.gateways.toml"));
    }
}
