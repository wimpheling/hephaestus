use crate::LocalWorkspaceManager;
use crate::common::{
    LocalWorkspaceError, OsStr, PathBuf, PreparedRuntimeGitWorkspace, RUNTIME_GIT_GUEST_PATH,
    RUNTIME_GIT_LOOPBACK_PORT, RepositoryId, Run, RunId, VmMount, WorkspaceId, WorkspaceMetadata,
    join_error, materialize_runtime_git, remove_owned_workspace, utf8_path,
};
use crate::errors::repository;

impl LocalWorkspaceManager {
    // Keep the durable workspace row, trusted Git setup, and failure cleanup
    // together so recovery observes one auditable lifecycle boundary.
    #[allow(clippy::too_many_lines)]
    /// Prepares an isolated runtime Git worktree for a run.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace lifecycle or persistence operation fails.
    pub(crate) async fn prepare_runtime_git_run(
        &self,
        run: &Run,
    ) -> Result<Option<PreparedRuntimeGitWorkspace>, LocalWorkspaceError> {
        let Some(request) = self
            .metadata
            .runtime_git_request(run.id)
            .await
            .map_err(repository)?
        else {
            return Ok(None);
        };
        request.validate().map_err(repository)?;
        let workspace_id = WorkspaceId::new();
        let owner = run.id.to_string();
        let active_path = self.config.workspace_root.join("active").join(&owner);
        let temporary_path = self
            .config
            .workspace_root
            .join("active")
            .join(format!("{owner}.runtime-git-preparing"));
        let active_text = utf8_path(&active_path)?;
        let sealed_path = self.config.workspace_root.join("sealed").join(&owner);
        let sealed_text = utf8_path(&sealed_path)?;
        self.metadata
            .insert_runtime_git_preparing(
                &WorkspaceMetadata {
                    id: workspace_id.as_uuid(),
                    state: String::from("preparing"),
                    active_path: active_text.clone(),
                    sealed_path: sealed_text,
                    input_commit: Some(request.target_commit.clone()),
                },
                request.repository_id,
                &request.target_commit,
                run.id,
                serde_json::json!({
                    "workspace_id": workspace_id.to_string(),
                    "repository_id": request.repository_id,
                    "target_ref": request.target_ref,
                    "target_commit": request.target_commit,
                }),
            )
            .await
            .map_err(repository)?;

        let repository_path = self.repository_path(RepositoryId::from_uuid(request.repository_id));
        let config = self.config.clone();
        let failed_temporary_path = temporary_path.clone();
        let materialization_request = request.clone();
        let active_path_for_materialization = active_path.clone();
        let materialized = tokio::task::spawn_blocking(move || {
            materialize_runtime_git(
                &config,
                &repository_path,
                &materialization_request,
                &temporary_path,
                &active_path_for_materialization,
            )
        })
        .await
        .map_err(join_error)?;
        let (tree, manifest_hash) = match materialized {
            Ok(value) => value,
            Err(error) => {
                if failed_temporary_path.exists() {
                    remove_owned_workspace(&self.config, &failed_temporary_path, "active")?;
                }
                self.metadata
                    .mark_materialization_failed(run.id, &error.to_string())
                    .await
                    .map_err(repository)?;
                return Err(error);
            }
        };
        self.metadata
            .mark_active(
                run.id,
                &tree,
                &manifest_hash,
                serde_json::json!({
                    "workspace_id": workspace_id.to_string(),
                    "repository_id": request.repository_id,
                    "target_ref": request.target_ref,
                    "target_commit": request.target_commit,
                    "guest_path": RUNTIME_GIT_GUEST_PATH,
                    "remote": "origin",
                }),
            )
            .await
            .map_err(repository)?;
        Ok(Some(PreparedRuntimeGitWorkspace {
            id: workspace_id,
            mount: VmMount {
                tag: String::from("runtime-git-worktree"),
                host_path: active_path,
                guest_path: PathBuf::from(RUNTIME_GIT_GUEST_PATH),
                read_only: false,
            },
            bridge: vm_trait::RuntimeGitBridge::new(
                request.repository_id,
                RUNTIME_GIT_LOOPBACK_PORT,
            ),
            target_ref: request.target_ref,
            target_commit: request.target_commit,
        }))
    }

    /// Removes an incomplete runtime Git worktree.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace lifecycle or persistence operation fails.
    pub(crate) async fn abandon_runtime_git_run(
        &self,
        run_id: RunId,
    ) -> Result<(), LocalWorkspaceError> {
        let row = self
            .metadata
            .runtime_git_workspace(run_id)
            .await
            .map_err(repository)?;
        let Some(row) = row else {
            return Ok(());
        };
        if row.state == "cleaned" {
            return Ok(());
        }
        let active = PathBuf::from(&row.active_path);
        let preparing = active.with_file_name(format!(
            "{}.runtime-git-preparing",
            active
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
        ));
        if preparing.exists() {
            remove_owned_workspace(&self.config, &preparing, "active")?;
        }
        for (path, class) in [
            (active, "active"),
            (PathBuf::from(row.sealed_path), "sealed"),
        ] {
            if path.exists() {
                remove_owned_workspace(&self.config, &path, class)?;
            }
        }
        self.metadata
            .set_state(run_id, "abandoned")
            .await
            .map_err(repository)?;
        self.metadata
            .event(run_id, "runtime_git.abandoned", serde_json::json!({}))
            .await
            .map_err(repository)
    }

    /// Cleans runtime Git worktrees left by interrupted runs.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace lifecycle or persistence operation fails.
    pub(crate) async fn recover_runtime_git_runs(&self) -> Result<usize, LocalWorkspaceError> {
        let rows = self
            .metadata
            .runtime_git_workspaces()
            .await
            .map_err(repository)?;
        let mut recovered = 0;
        for (run_id, row) in rows {
            let active = PathBuf::from(&row.active_path);
            let preparing = active.with_file_name(format!(
                "{}.runtime-git-preparing",
                active
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or_default()
            ));
            if preparing.exists() {
                remove_owned_workspace(&self.config, &preparing, "active")?;
            }
            for (path, class) in [
                (active, "active"),
                (PathBuf::from(row.sealed_path), "sealed"),
            ] {
                if path.exists() {
                    remove_owned_workspace(&self.config, &path, class)?;
                }
            }
            self.metadata
                .mark_cleaned(run_id)
                .await
                .map_err(repository)?;
            recovered += 1;
        }
        Ok(recovered)
    }
}
