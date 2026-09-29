use crate::LocalWorkspaceManager;
use crate::common::{
    LocalWorkspaceError, OsStr, Path, PathBuf, PreparedWorkspace, Run, SOURCE_GUEST_PATH, VmMount,
    WORK_GUEST_PATH, WorkspaceId, WorkspaceMetadata, ensure_workspace_path, join_error,
    materialize, remove_owned_workspace, repository, utf8_path, validate_mount_policy,
};

impl LocalWorkspaceManager {
    // Keeping the ordered materialization state changes together makes cleanup
    // and crash boundaries directly auditable.
    #[allow(clippy::too_many_lines)]
    /// Prepares the source and writable workspace mounts for a run.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace lifecycle or persistence operation fails.
    pub(crate) async fn prepare_run(
        &self,
        run: &Run,
    ) -> Result<PreparedWorkspace, LocalWorkspaceError> {
        let Some(request) = self.request(run).await? else {
            return Ok(PreparedWorkspace::disabled());
        };
        if !request.config.workspace.mount {
            return Ok(PreparedWorkspace::disabled());
        }
        validate_mount_policy(&request.config)?;

        if let Some(row) = self.metadata.workspace(run.id).await.map_err(repository)? {
            if row.state == "active" {
                return self.prepared_from_row(&row);
            }
            return Err(LocalWorkspaceError::State(format!(
                "workspace for run {} is already {}",
                run.id, row.state
            )));
        }

        let workspace_id = WorkspaceId::new();
        let active_path = self.active_path(run.id);
        let sealed_path = self.sealed_path(run.id);
        let active_text = utf8_path(&active_path)?;
        let sealed_text = utf8_path(&sealed_path)?;
        self.metadata
            .insert_preparing(
                &WorkspaceMetadata {
                    id: workspace_id.as_uuid(),
                    state: String::from("preparing"),
                    active_path: active_text.clone(),
                    sealed_path: sealed_text.clone(),
                    input_commit: Some(request.commit.clone()),
                },
                request.repository_id.as_uuid(),
                &request.commit,
                run.id,
            )
            .await
            .map_err(repository)?;

        let temporary_path = self
            .config
            .workspace_root
            .join("active")
            .join(format!("{}.{}", run.id, workspace_id));
        let repository_path = self.repository_path(request.repository_id);
        let config = self.config.clone();
        let commit = request.commit.clone();
        let failed_temporary_path = temporary_path.clone();
        let materialized = tokio::task::spawn_blocking(move || {
            materialize(
                &config,
                &repository_path,
                &commit,
                &temporary_path,
                &active_path,
            )
        })
        .await
        .map_err(join_error)?;
        let materialized = match materialized {
            Ok(value) => value,
            Err(error) => {
                if failed_temporary_path
                    .join(".hephaestus-workspace")
                    .is_file()
                {
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
                &materialized.tree,
                &materialized.manifest_hash,
                serde_json::json!({
                    "workspace_id": workspace_id.to_string(),
                    "repository_id": request.repository_id,
                    "input_commit": request.commit,
                    "input_tree": materialized.tree,
                    "materialization_hash": materialized.manifest_hash,
                    "source_mount": SOURCE_GUEST_PATH,
                    "work_mount": WORK_GUEST_PATH,
                }),
            )
            .await
            .map_err(repository)?;
        self.prepared(workspace_id, &self.active_path(run.id))
    }

    fn prepared_from_row(
        &self,
        row: &WorkspaceMetadata,
    ) -> Result<PreparedWorkspace, LocalWorkspaceError> {
        let active = PathBuf::from(&row.active_path);
        let expected =
            self.config
                .workspace_root
                .join("active")
                .join(active.file_name().ok_or_else(|| {
                    LocalWorkspaceError::UnsafePath(String::from(
                        "active workspace has no file name",
                    ))
                })?);
        if active != expected
            || row.sealed_path != utf8_path(&self.sealed_path_from_active(&active))?
        {
            return Err(LocalWorkspaceError::UnsafePath(String::from(
                "stored workspace paths do not use the canonical layout",
            )));
        }
        self.prepared(WorkspaceId::from_uuid(row.id), &active)
    }

    fn sealed_path_from_active(&self, active: &Path) -> PathBuf {
        self.config
            .workspace_root
            .join("sealed")
            .join(active.file_name().unwrap_or_else(|| OsStr::new("invalid")))
    }

    fn prepared(
        &self,
        workspace_id: WorkspaceId,
        active: &Path,
    ) -> Result<PreparedWorkspace, LocalWorkspaceError> {
        ensure_workspace_path(&self.config, active, "active")?;
        Ok(PreparedWorkspace {
            id: Some(workspace_id),
            mounts: vec![
                VmMount {
                    tag: String::from("repository-source"),
                    host_path: active.join("source"),
                    guest_path: PathBuf::from(SOURCE_GUEST_PATH),
                    read_only: true,
                },
                VmMount {
                    tag: String::from("repository-work"),
                    host_path: active.join("work"),
                    guest_path: PathBuf::from(WORK_GUEST_PATH),
                    read_only: false,
                },
            ],
        })
    }
}
