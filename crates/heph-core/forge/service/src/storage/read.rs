//! Immutable Git object reads and input validation.

use super::{GIT_COMMAND_TIMEOUT, GitStorage, GitStorageError};
use std::process::Stdio;
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

impl GitStorage {
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
        repository_id: forge_domain::RepositoryId,
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
}

pub(super) fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn valid_relative_path(value: &str) -> bool {
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
