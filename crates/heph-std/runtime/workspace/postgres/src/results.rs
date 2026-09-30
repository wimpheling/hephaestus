//! Workspace result `PostgreSQL` repository.

use async_trait::async_trait;
use runtime_types::RunId;
use serde_json::Value;
use uuid::Uuid;
use workspace_domain::{
    PendingResultMetadata, ResultArtifactMetadata, ResultId, ResultMetadata, ResultRepository,
    WorkspaceRepositoryError,
};

use super::error::error;
use super::repository::PgWorkspaceMetadataRepository;
use super::rows::{PendingRow, ResultRow};

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
