//! Workspace metadata `PostgreSQL` repository.

use async_trait::async_trait;
use runtime_types::RunId;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;
use workspace_domain::{
    RuntimeGitWorkspaceRequest, WorkspaceMetadata, WorkspaceMetadataRepository,
    WorkspaceRepositoryError, WorkspaceRequestMetadata,
};

use super::error::error;
use super::rows::{
    RequestRow, RuntimeGitRequestRow, RuntimeGitWorkspaceRow, WorkspaceRow, insert_event,
};

/// `PostgreSQL` implementation of workspace metadata persistence.
#[derive(Clone)]
pub struct PgWorkspaceMetadataRepository {
    pub(super) pool: PgPool,
}

impl PgWorkspaceMetadataRepository {
    /// Creates an adapter over a shared pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorkspaceMetadataRepository for PgWorkspaceMetadataRepository {
    async fn request(
        &self,
        command_id: Uuid,
    ) -> Result<Option<WorkspaceRequestMetadata>, WorkspaceRepositoryError> {
        let row = sqlx::query_as::<_, RequestRow>(
            "SELECT request.repository_id, request.commit_sha, request.instance_id, release.configuration AS configuration
             FROM run_requests request JOIN releases release ON release.id = request.release_id
             WHERE request.command_id = $1 AND request.dispatch_state <> 'denied'
             UNION ALL
             SELECT attachment.repository_id, attempt.target_commit, run.instance_id, release.configuration AS configuration
             FROM mailbox_delivery_attempts attempt
             JOIN runs run ON run.id = attempt.run_id
             JOIN agent_attachments attachment ON attachment.id = run.attachment_id
               AND attachment.instance_id = run.instance_id
             JOIN releases release ON release.id = run.release_id
             WHERE attempt.command_id = $1 AND attempt.target_commit IS NOT NULL",
        ).bind(command_id).fetch_optional(&self.pool).await.map_err(error)?;
        Ok(row.map(Into::into))
    }

    async fn workspace(
        &self,
        run_id: RunId,
    ) -> Result<Option<WorkspaceMetadata>, WorkspaceRepositoryError> {
        let row = sqlx::query_as::<_, WorkspaceRow>("SELECT id, state, active_path, sealed_path, input_commit FROM run_workspaces WHERE run_id = $1")
            .bind(run_id.as_uuid()).fetch_optional(&self.pool).await.map_err(error)?;
        Ok(row.map(Into::into))
    }

    async fn insert_preparing(
        &self,
        metadata: &WorkspaceMetadata,
        repository_id: Uuid,
        input_commit: &str,
        run_id: RunId,
    ) -> Result<(), WorkspaceRepositoryError> {
        sqlx::query("INSERT INTO run_workspaces (id, run_id, repository_id, input_commit, active_path, sealed_path, state) VALUES ($1, $2, $3, $4, $5, $6, 'preparing')")
            .bind(metadata.id).bind(run_id.as_uuid()).bind(repository_id).bind(input_commit).bind(&metadata.active_path).bind(&metadata.sealed_path)
            .execute(&self.pool).await.map_err(error).map(|_| ())
    }

    async fn mark_materialization_failed(
        &self,
        run_id: RunId,
        message: &str,
    ) -> Result<(), WorkspaceRepositoryError> {
        sqlx::query("UPDATE run_workspaces SET state = 'materialization_failed', failure = jsonb_build_object('message', $2) WHERE run_id = $1")
            .bind(run_id.as_uuid()).bind(message).execute(&self.pool).await.map_err(error).map(|_| ())
    }

    async fn mark_active(
        &self,
        run_id: RunId,
        input_tree: &str,
        manifest_hash: &str,
        event: Value,
    ) -> Result<(), WorkspaceRepositoryError> {
        let mut tx = self.pool.begin().await.map_err(error)?;
        sqlx::query("UPDATE run_workspaces SET state = 'active', input_tree = $2, materialization_hash = $3 WHERE run_id = $1 AND state = 'preparing'")
            .bind(run_id.as_uuid()).bind(input_tree).bind(manifest_hash).execute(&mut *tx).await.map_err(error)?;
        insert_event(&mut tx, run_id, "workspace.active", event).await?;
        tx.commit().await.map_err(error)
    }

    async fn event(
        &self,
        run_id: RunId,
        event_type: &str,
        payload: Value,
    ) -> Result<(), WorkspaceRepositoryError> {
        let mut tx = self.pool.begin().await.map_err(error)?;
        insert_event(&mut tx, run_id, event_type, payload).await?;
        tx.commit().await.map_err(error)
    }

    async fn set_state(&self, run_id: RunId, state: &str) -> Result<(), WorkspaceRepositoryError> {
        sqlx::query("UPDATE run_workspaces SET state = $2 WHERE run_id = $1")
            .bind(run_id.as_uuid())
            .bind(state)
            .execute(&self.pool)
            .await
            .map_err(error)
            .map(|_| ())
    }

    async fn insert_runtime_git_preparing(
        &self,
        metadata: &WorkspaceMetadata,
        repository_id: Uuid,
        input_commit: &str,
        run_id: RunId,
        event: Value,
    ) -> Result<(), WorkspaceRepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(error)?;
        sqlx::query("INSERT INTO run_workspaces (id, run_id, repository_id, input_commit, active_path, sealed_path, state) VALUES ($1, $2, $3, $4, $5, $6, 'preparing')")
            .bind(metadata.id)
            .bind(run_id.as_uuid())
            .bind(repository_id)
            .bind(input_commit)
            .bind(&metadata.active_path)
            .bind(&metadata.sealed_path)
            .execute(&mut *transaction)
            .await
            .map_err(error)?;
        insert_event(&mut transaction, run_id, "runtime_git.preparing", event).await?;
        transaction.commit().await.map_err(error)
    }

    async fn mark_failed(
        &self,
        run_id: RunId,
        state: &str,
        message: &str,
    ) -> Result<(), WorkspaceRepositoryError> {
        sqlx::query("UPDATE run_workspaces SET state = $2, failure = jsonb_build_object('message', $3) WHERE run_id = $1").bind(run_id.as_uuid()).bind(state).bind(message).execute(&self.pool).await.map_err(error).map(|_| ())
    }

    async fn mark_cleaned(&self, run_id: RunId) -> Result<(), WorkspaceRepositoryError> {
        sqlx::query("UPDATE run_workspaces SET state = 'cleaned', cleaned_at = now() WHERE run_id = $1 AND state <> 'cleaned'").bind(run_id.as_uuid()).execute(&self.pool).await.map_err(error).map(|_| ())
    }

    async fn runtime_git_request(
        &self,
        run_id: RunId,
    ) -> Result<Option<RuntimeGitWorkspaceRequest>, WorkspaceRepositoryError> {
        let row = sqlx::query_as::<_, RuntimeGitRequestRow>(
            "SELECT git.repository_id,
                    COALESCE(provenance.target_repository_id, $2) AS target_repository_id,
                    COALESCE(provenance.target_ref, '') AS target_ref,
                    COALESCE(provenance.target_commit, '') AS target_commit,
                    git.git_operations, git.ref_globs
             FROM runs AS run
             JOIN run_authorization_snapshots AS snapshot
               ON snapshot.run_id = run.id
              AND snapshot.instance_id = run.instance_id
              AND snapshot.instance_revision_id = run.instance_revision_id
             JOIN run_git_authority_snapshots AS git
               ON git.snapshot_id = snapshot.id
              AND git.instance_revision_id = snapshot.instance_revision_id
             LEFT JOIN run_instance_provenance AS provenance
               ON provenance.run_id = run.id
              AND provenance.instance_id = run.instance_id
              AND provenance.instance_revision_id = run.instance_revision_id
             WHERE run.id = $1",
        )
        .bind(run_id.as_uuid())
        .bind(Uuid::nil())
        .fetch_optional(&self.pool)
        .await
        .map_err(error)?;
        Ok(row.map(Into::into))
    }

    async fn runtime_git_workspace(
        &self,
        run_id: RunId,
    ) -> Result<Option<WorkspaceMetadata>, WorkspaceRepositoryError> {
        let row = sqlx::query_as::<_, WorkspaceRow>(
            "SELECT workspace.id, workspace.state, workspace.active_path,
                    workspace.sealed_path, workspace.input_commit
             FROM run_workspaces AS workspace
             WHERE workspace.run_id = $1
               AND workspace.state IN ('preparing', 'active', 'materialization_failed')
               AND EXISTS (
                   SELECT 1 FROM run_events AS event
                   WHERE event.run_id = workspace.run_id
                     AND event.event_type = 'runtime_git.preparing'
               )",
        )
        .bind(run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(error)?;
        Ok(row.map(Into::into))
    }

    async fn runtime_git_workspaces(
        &self,
    ) -> Result<Vec<(RunId, WorkspaceMetadata)>, WorkspaceRepositoryError> {
        let rows = sqlx::query_as::<_, RuntimeGitWorkspaceRow>(
            "SELECT workspace.run_id, workspace.id, workspace.state,
                    workspace.active_path, workspace.sealed_path, workspace.input_commit
             FROM run_workspaces AS workspace
             WHERE workspace.state IN ('preparing', 'active', 'materialization_failed')
               AND EXISTS (
                   SELECT 1 FROM run_events AS event
                   WHERE event.run_id = workspace.run_id
                     AND event.event_type = 'runtime_git.preparing'
               )",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(error)?;
        Ok(rows
            .into_iter()
            .map(|row| (RunId::from_uuid(row.run_id), row.into()))
            .collect())
    }
}
