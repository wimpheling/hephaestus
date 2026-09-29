use crate::LocalWorkspaceManager;
use crate::common::AgentInstanceId;
use crate::common::{
    AgentAttachmentId, AgentInstanceRevisionId, CommandId, LocalWorkspaceError, PathBuf,
    ReleaseAgentId, ReleaseId, RepositoryId, ResultId, Run, RunId, RunState, cas_publish_ref,
    parse_run_kind, remove_owned_workspace, repository,
};

impl LocalWorkspaceManager {
    /// Abandons and removes an incomplete workspace.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace lifecycle or persistence operation fails.
    pub(crate) async fn abandon_run(&self, run_id: RunId) -> Result<(), LocalWorkspaceError> {
        let row = self.metadata.workspace(run_id).await.map_err(repository)?;
        let Some(row) = row else {
            return Ok(());
        };
        if row.state == "cleaned" {
            return Ok(());
        }
        let active = PathBuf::from(row.active_path);
        if active.exists() {
            remove_owned_workspace(&self.config, &active, "active")?;
        }
        let sealed = PathBuf::from(row.sealed_path);
        if sealed.exists() {
            remove_owned_workspace(&self.config, &sealed, "sealed")?;
        }
        self.metadata
            .set_state(run_id, "abandoned")
            .await
            .map_err(repository)?;
        self.metadata
            .event(run_id, "workspace.abandoned", serde_json::json!({}))
            .await
            .map_err(repository)
    }

    /// Replays durable pending and prepared result states after restart.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace lifecycle or persistence operation fails.
    pub(crate) async fn recover_incomplete(&self) -> Result<usize, LocalWorkspaceError> {
        let pending = self.results.pending().await.map_err(repository)?;
        let mut recovered = 0;
        for row in pending {
            let run = Run {
                id: RunId::from_uuid(row.run_id),
                instance_id: AgentInstanceId::from_uuid(row.instance_id),
                instance_revision_id: AgentInstanceRevisionId::from_uuid(row.instance_revision_id),
                release_id: ReleaseId::from_uuid(row.release_id),
                release_agent_id: ReleaseAgentId::from_uuid(row.release_agent_id),
                attachment_id: row.attachment_id.map(AgentAttachmentId::from_uuid),
                kind: parse_run_kind(&row.run_kind)?,
                command_id: CommandId::from_uuid(row.command_id),
                requires_state: row.requires_state,
                volume_id: None,
                lease_id: None,
                lease_fencing_token: None,
                vm_id: None,
                state: RunState::Running,
                outcome: None,
                exit: None,
                failure: None,
                cancel_requested_at: None,
                created_at: row.created_at,
                updated_at: row.created_at,
                state_version: 0,
            };
            self.finalize_run(&run, &row.message).await?;
            recovered += 1;
        }
        let rows = self.results.prepared().await.map_err(repository)?;
        for row in rows {
            let Some(commit) = row.result_commit else {
                continue;
            };
            let repository_path = self.repository_path(RepositoryId::from_uuid(row.repository_id));
            cas_publish_ref(&self.config, &repository_path, &row.result_ref, &commit)?;
            self.results
                .mark_completed(ResultId::from_uuid(row.id))
                .await
                .map_err(repository)?;
            self.metadata
                .event(
                    RunId::from_uuid(row.run_id),
                    "result.completed",
                    serde_json::json!({"recovered": true, "result_commit": commit}),
                )
                .await
                .map_err(repository)?;
            self.cleanup_completed_workspace(RunId::from_uuid(row.run_id))
                .await?;
            recovered += 1;
        }
        Ok(recovered)
    }

    /// Removes workspace trees after a result reaches a terminal state.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace lifecycle or persistence operation fails.
    pub(crate) async fn cleanup_completed_workspace(
        &self,
        run_id: RunId,
    ) -> Result<(), LocalWorkspaceError> {
        let workspace = self.metadata.workspace(run_id).await.map_err(repository)?;
        let Some(workspace) = workspace else {
            return Ok(());
        };
        if workspace.state == "cleaned" {
            return Ok(());
        }
        for (path, class) in [
            (PathBuf::from(workspace.active_path), "active"),
            (PathBuf::from(workspace.sealed_path), "sealed"),
        ] {
            if path.exists() {
                remove_owned_workspace(&self.config, &path, class)?;
            }
        }
        self.metadata
            .mark_cleaned(run_id)
            .await
            .map_err(repository)?;
        self.metadata
            .event(run_id, "workspace.cleaned", serde_json::json!({}))
            .await
            .map_err(repository)
    }
}
