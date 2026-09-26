use crate::LocalWorkspaceManager;
use crate::common::{
    LocalWorkspaceConfig, LocalWorkspaceError, PathBuf, PublishedResult, RepositoryId,
    ResultRepository, Run, RunId, RunRequest, WorkspaceMetadataRepository, fs, io_error,
    published_result, repository,
};
use std::sync::Arc;

impl LocalWorkspaceManager {
    /// Creates a manager after validating its configured roots.
    ///
    /// # Errors
    ///
    /// Returns an error when a path is relative, overlaps another storage
    /// class, or the Git executable is not an absolute regular file.
    pub fn new(
        metadata: Arc<dyn WorkspaceMetadataRepository>,
        results: Arc<dyn ResultRepository>,
        config: LocalWorkspaceConfig,
    ) -> Result<Self, LocalWorkspaceError> {
        for (name, path) in [
            ("workspace_root", &config.workspace_root),
            ("artifact_root", &config.artifact_root),
            ("repository_root", &config.repository_root),
            ("git_binary", &config.git_binary),
        ] {
            if !path.is_absolute() {
                return Err(LocalWorkspaceError::Configuration(format!(
                    "{name} must be absolute"
                )));
            }
        }
        for (left_name, left, right_name, right) in [
            (
                "workspace_root",
                &config.workspace_root,
                "artifact_root",
                &config.artifact_root,
            ),
            (
                "workspace_root",
                &config.workspace_root,
                "repository_root",
                &config.repository_root,
            ),
            (
                "artifact_root",
                &config.artifact_root,
                "repository_root",
                &config.repository_root,
            ),
        ] {
            if left.starts_with(right) || right.starts_with(left) {
                return Err(LocalWorkspaceError::Configuration(format!(
                    "{left_name} and {right_name} must not overlap"
                )));
            }
        }
        if config.limits.max_entries == 0
            || config.limits.max_file_bytes == 0
            || config.limits.max_total_bytes == 0
            || config.limits.max_patch_bytes == 0
        {
            return Err(LocalWorkspaceError::Configuration(String::from(
                "workspace limits must be greater than zero",
            )));
        }
        Ok(Self {
            metadata,
            results,
            config,
        })
    }

    /// Creates and canonicalizes workspace and artifact storage roots.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe roots or inaccessible storage.
    pub fn initialize(&mut self) -> Result<(), LocalWorkspaceError> {
        fs::create_dir_all(self.config.workspace_root.join("active")).map_err(io_error)?;
        fs::create_dir_all(self.config.workspace_root.join("sealed")).map_err(io_error)?;
        fs::create_dir_all(&self.config.artifact_root).map_err(io_error)?;
        self.config.workspace_root =
            fs::canonicalize(&self.config.workspace_root).map_err(io_error)?;
        self.config.artifact_root =
            fs::canonicalize(&self.config.artifact_root).map_err(io_error)?;
        self.config.repository_root =
            fs::canonicalize(&self.config.repository_root).map_err(io_error)?;
        for (left_name, left, right_name, right) in [
            (
                "workspace_root",
                &self.config.workspace_root,
                "artifact_root",
                &self.config.artifact_root,
            ),
            (
                "workspace_root",
                &self.config.workspace_root,
                "repository_root",
                &self.config.repository_root,
            ),
            (
                "artifact_root",
                &self.config.artifact_root,
                "repository_root",
                &self.config.repository_root,
            ),
        ] {
            if left.starts_with(right) || right.starts_with(left) {
                return Err(LocalWorkspaceError::Configuration(format!(
                    "canonical {left_name} and {right_name} must not overlap"
                )));
            }
        }
        let git = fs::canonicalize(&self.config.git_binary).map_err(io_error)?;
        if !git.is_file() {
            return Err(LocalWorkspaceError::Configuration(String::from(
                "git_binary must be a regular file",
            )));
        }
        self.config.git_binary = git;
        Ok(())
    }

    /// Returns the canonical active path for a run.
    #[must_use]
    pub(crate) fn active_path(&self, run_id: RunId) -> PathBuf {
        self.config
            .workspace_root
            .join("active")
            .join(run_id.to_string())
    }

    /// Returns the canonical sealed path for a run.
    #[must_use]
    pub(crate) fn sealed_path(&self, run_id: RunId) -> PathBuf {
        self.config
            .workspace_root
            .join("sealed")
            .join(run_id.to_string())
    }

    /// Returns the canonical bare repository path.
    #[must_use]
    pub(crate) fn repository_path(&self, repository_id: RepositoryId) -> PathBuf {
        self.config
            .repository_root
            .join(format!("{repository_id}.git"))
    }

    /// Loads and validates the persisted request associated with a run.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace lifecycle or persistence operation fails.
    pub(crate) async fn request(
        &self,
        run: &Run,
    ) -> Result<Option<RunRequest>, LocalWorkspaceError> {
        let row = self
            .metadata
            .request(run.command_id.as_uuid())
            .await
            .map_err(repository)?;
        row.map(TryInto::try_into).transpose()
    }

    /// Loads a completed result when one has already been published.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace lifecycle or persistence operation fails.
    pub(crate) async fn existing_result(
        &self,
        run_id: RunId,
    ) -> Result<Option<PublishedResult>, LocalWorkspaceError> {
        let row = self.results.completed(run_id).await.map_err(repository)?;
        row.map(published_result).transpose()
    }
}
