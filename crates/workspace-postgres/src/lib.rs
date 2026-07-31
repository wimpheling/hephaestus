//! `PostgreSQL` persistence adapter for workspace metadata and results.

use async_trait::async_trait;
use runtime_types::RunId;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;
use workspace_domain::{
    PendingResultMetadata, ResultArtifactMetadata, ResultId, ResultMetadata, ResultRepository,
    WorkspaceMetadata, WorkspaceMetadataRepository, WorkspaceRepositoryError,
    WorkspaceRequestMetadata,
};

/// `PostgreSQL` implementation of workspace metadata persistence.
#[derive(Clone)]
pub struct PgWorkspaceMetadataRepository {
    pool: PgPool,
}

impl PgWorkspaceMetadataRepository {
    /// Creates an adapter over a shared pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn error(error: impl std::fmt::Display) -> WorkspaceRepositoryError {
    WorkspaceRepositoryError::new(error.to_string())
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
             WHERE request.command_id = $1 AND request.dispatch_state <> 'denied'",
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
}

#[async_trait]
impl ResultRepository for PgWorkspaceMetadataRepository {
    async fn vm_logs(
        &self,
        run_id: RunId,
    ) -> Result<(Vec<Value>, Option<Value>), WorkspaceRepositoryError> {
        let logs = sqlx::query_scalar("SELECT payload FROM run_events WHERE run_id=$1 AND event_type='vm.log' ORDER BY sequence").bind(run_id.as_uuid()).fetch_all(&self.pool).await.map_err(error)?;
        let exit = sqlx::query_scalar("SELECT payload FROM run_events WHERE run_id=$1 AND event_type='vm.exited' ORDER BY sequence DESC LIMIT 1").bind(run_id.as_uuid()).fetch_optional(&self.pool).await.map_err(error)?;
        Ok((logs, exit))
    }
    async fn completed(
        &self,
        run_id: RunId,
    ) -> Result<Option<ResultMetadata>, WorkspaceRepositoryError> {
        let row = sqlx::query_as::<_, ResultRow>("SELECT id, run_id, repository_id, result_ref, result_commit, result_tree FROM run_results WHERE run_id = $1 AND state = 'completed'").bind(run_id.as_uuid()).fetch_optional(&self.pool).await.map_err(error)?;
        Ok(row.map(Into::into))
    }

    async fn insert_pending(
        &self,
        run_id: RunId,
        repository_id: Uuid,
        instance_id: Uuid,
        instance_revision_id: Uuid,
        release_id: Uuid,
        release_agent_id: Uuid,
        input_commit: &str,
        result_ref: &str,
        message: &str,
    ) -> Result<(), WorkspaceRepositoryError> {
        sqlx::query("INSERT INTO run_results (id, run_id, repository_id, instance_id, instance_revision_id, release_id, release_agent_id, input_commit, result_ref, message, state) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'pending') ON CONFLICT (run_id) DO NOTHING")
            .bind(Uuid::new_v4()).bind(run_id.as_uuid()).bind(repository_id).bind(instance_id).bind(instance_revision_id).bind(release_id).bind(release_agent_id).bind(input_commit).bind(result_ref).bind(message).execute(&self.pool).await.map_err(error).map(|_| ())
    }

    async fn reject(&self, run_id: RunId, message: &str) -> Result<(), WorkspaceRepositoryError> {
        sqlx::query("UPDATE run_results SET state = 'rejected', diagnostics = jsonb_build_array(jsonb_build_object('message', $2)) WHERE run_id = $1").bind(run_id.as_uuid()).bind(message).execute(&self.pool).await.map_err(error).map(|_| ())
    }

    async fn id_for_run(&self, run_id: RunId) -> Result<ResultId, WorkspaceRepositoryError> {
        let id: Uuid = sqlx::query_scalar("SELECT id FROM run_results WHERE run_id = $1")
            .bind(run_id.as_uuid())
            .fetch_one(&self.pool)
            .await
            .map_err(error)?;
        Ok(ResultId::from_uuid(id))
    }

    async fn persist_prepared(
        &self,
        result_id: ResultId,
        _run_id: RunId,
        tree: &str,
        commit: &str,
        manifest_hash: &str,
        artifacts: &[ResultArtifactMetadata],
    ) -> Result<(), WorkspaceRepositoryError> {
        let mut tx = self.pool.begin().await.map_err(error)?;
        sqlx::query("UPDATE run_results SET state = 'prepared', result_tree = $2, result_commit = $3, artifact_manifest_hash = $4, prepared_at = now() WHERE id = $1 AND state IN ('pending','prepared')").bind(result_id.as_uuid()).bind(tree).bind(commit).bind(manifest_hash).execute(&mut *tx).await.map_err(error)?;
        for artifact in artifacts {
            sqlx::query("INSERT INTO result_artifacts (id,result_id,kind,path,git_mode,media_type,size_bytes,sha256,storage_key,provenance) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,jsonb_build_object('generated_by','workspace-local')) ON CONFLICT (result_id,kind,path) DO NOTHING")
                .bind(artifact.id).bind(result_id.as_uuid()).bind(&artifact.kind).bind(&artifact.path).bind(artifact.git_mode).bind(&artifact.media_type).bind(artifact.size_bytes).bind(&artifact.sha256).bind(&artifact.storage_key).execute(&mut *tx).await.map_err(error)?;
        }
        tx.commit().await.map_err(error)
    }

    async fn mark_ref_published(
        &self,
        result_id: ResultId,
        commit: &str,
    ) -> Result<(), WorkspaceRepositoryError> {
        sqlx::query("UPDATE run_results SET state = 'ref_published', published_at = now() WHERE id = $1 AND result_commit = $2").bind(result_id.as_uuid()).bind(commit).execute(&self.pool).await.map_err(error).map(|_| ())
    }

    async fn mark_completed(&self, result_id: ResultId) -> Result<(), WorkspaceRepositoryError> {
        sqlx::query("UPDATE run_results SET state = 'completed', completed_at = now() WHERE id = $1 AND state = 'ref_published'").bind(result_id.as_uuid()).execute(&self.pool).await.map_err(error).map(|_| ())
    }

    async fn pending(&self) -> Result<Vec<PendingResultMetadata>, WorkspaceRepositoryError> {
        let rows = sqlx::query_as::<_, PendingRow>("SELECT result.run_id,result.message,run.command_id,run.instance_id,run.instance_revision_id,run.release_id,run.release_agent_id,run.attachment_id,run.run_kind,run.requires_state,run.created_at FROM run_results result JOIN runs run ON run.id=result.run_id WHERE result.state='pending'").fetch_all(&self.pool).await.map_err(error)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn prepared(&self) -> Result<Vec<ResultMetadata>, WorkspaceRepositoryError> {
        let rows = sqlx::query_as::<_, ResultRow>("SELECT id,run_id,repository_id,result_ref,result_commit,result_tree FROM run_results WHERE state IN ('prepared','ref_published')").fetch_all(&self.pool).await.map_err(error)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}

async fn insert_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: RunId,
    event_type: &str,
    payload: Value,
) -> Result<(), WorkspaceRepositoryError> {
    sqlx::query("SELECT id FROM runs WHERE id = $1 FOR UPDATE")
        .bind(run_id.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(error)?;
    let sequence: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sequence),0)+1 FROM run_events WHERE run_id=$1")
            .bind(run_id.as_uuid())
            .fetch_one(&mut **tx)
            .await
            .map_err(error)?;
    sqlx::query("INSERT INTO run_events (id,run_id,sequence,event_type,payload,occurred_at) VALUES ($1,$2,$3,$4,$5,now())").bind(Uuid::new_v4()).bind(run_id.as_uuid()).bind(sequence).bind(event_type).bind(payload).execute(&mut **tx).await.map_err(error)?;
    Ok(())
}

#[derive(FromRow)]
struct RequestRow {
    repository_id: Uuid,
    commit_sha: String,
    instance_id: Uuid,
    configuration: Value,
}
impl From<RequestRow> for WorkspaceRequestMetadata {
    fn from(row: RequestRow) -> Self {
        Self {
            repository_id: row.repository_id,
            commit_sha: row.commit_sha,
            instance_id: row.instance_id,
            configuration: row.configuration,
        }
    }
}
#[derive(FromRow)]
struct WorkspaceRow {
    id: Uuid,
    state: String,
    active_path: String,
    sealed_path: String,
    input_commit: Option<String>,
}
impl From<WorkspaceRow> for WorkspaceMetadata {
    fn from(row: WorkspaceRow) -> Self {
        Self {
            id: row.id,
            state: row.state,
            active_path: row.active_path,
            sealed_path: row.sealed_path,
            input_commit: row.input_commit,
        }
    }
}
#[derive(FromRow)]
struct ResultRow {
    id: Uuid,
    run_id: Uuid,
    repository_id: Uuid,
    result_ref: String,
    result_commit: Option<String>,
    result_tree: Option<String>,
}
impl From<ResultRow> for ResultMetadata {
    fn from(row: ResultRow) -> Self {
        Self {
            id: row.id,
            run_id: row.run_id,
            repository_id: row.repository_id,
            result_ref: row.result_ref,
            result_commit: row.result_commit,
            result_tree: row.result_tree,
        }
    }
}
#[derive(FromRow)]
struct PendingRow {
    run_id: Uuid,
    message: String,
    command_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Option<Uuid>,
    run_kind: String,
    requires_state: bool,
    created_at: time::OffsetDateTime,
}
impl From<PendingRow> for PendingResultMetadata {
    fn from(row: PendingRow) -> Self {
        Self {
            run_id: row.run_id,
            message: row.message,
            command_id: row.command_id,
            instance_id: row.instance_id,
            instance_revision_id: row.instance_revision_id,
            release_id: row.release_id,
            release_agent_id: row.release_agent_id,
            attachment_id: row.attachment_id,
            run_kind: row.run_kind,
            requires_state: row.requires_state,
            created_at: row.created_at,
        }
    }
}
