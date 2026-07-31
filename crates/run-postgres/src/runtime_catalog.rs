use async_trait::async_trait;
use run_domain::{Run, RunKind};
use run_orchestrator::{
    RunRuntimeArtifact, RunRuntimeArtifactKind, RunRuntimeCatalog, RunRuntimeCatalogError,
    RunRuntimeInput,
};
use runtime_types::RunId;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::PgRunRepository;

#[async_trait]
impl RunRuntimeCatalog for PgRunRepository {
    async fn load_runtime(&self, run: &Run) -> Result<RunRuntimeInput, RunRuntimeCatalogError> {
        let context = sqlx::query_as::<_, RuntimeContextRow>(
            "SELECT revision.parameters,
                    request.repository_id, request.git_ref, request.commit_sha,
                    release.state AS release_state,
                    update.id AS update_id,
                    update.expected_current_revision_id AS previous_revision_id,
                    previous_agent.release_id AS previous_release_id,
                    previous.parameters AS previous_parameters
             FROM runs AS stored_run
             JOIN agent_instance_revisions AS revision
               ON revision.id = stored_run.instance_revision_id
              AND revision.instance_id = stored_run.instance_id
             JOIN release_agents AS release_agent
               ON release_agent.id = stored_run.release_agent_id
              AND release_agent.release_id = stored_run.release_id
              AND revision.release_agent_id = release_agent.id
             JOIN releases AS release ON release.id = stored_run.release_id
             LEFT JOIN run_requests AS request ON request.run_id = stored_run.id
             LEFT JOIN agent_updates AS update
               ON update.hook_run_id = stored_run.id
             LEFT JOIN agent_instance_revisions AS previous
               ON previous.id = update.expected_current_revision_id
              AND previous.instance_id = stored_run.instance_id
             LEFT JOIN release_agents AS previous_agent
               ON previous_agent.id = previous.release_agent_id
             WHERE stored_run.id = $1
               AND stored_run.instance_id = $2
               AND stored_run.instance_revision_id = $3
               AND stored_run.release_id = $4
               AND stored_run.release_agent_id = $5
               AND stored_run.run_kind = $6",
        )
        .bind(run.id.as_uuid())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .bind(run_kind_name(run.kind))
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(RunRuntimeCatalogError::Unavailable)?;

        if context.release_state != "published" {
            return Err(RunRuntimeCatalogError::InvalidData(
                "release is not published",
            ));
        }
        if run.kind == RunKind::Normal
            && (context.repository_id.is_none()
                || context.git_ref.is_none()
                || context.commit_sha.is_none())
        {
            return Err(RunRuntimeCatalogError::InvalidData(
                "normal run target provenance",
            ));
        }

        let artifacts = self
            .load_runtime_artifacts(run.release_id.as_uuid())
            .await?;
        let previous_artifacts = match context.previous_release_id {
            Some(release_id) => self.load_runtime_artifacts(release_id).await?,
            None => Vec::new(),
        };

        Ok(RunRuntimeInput {
            parameters: context.parameters,
            repository_id: context.repository_id,
            git_ref: context.git_ref,
            commit_sha: context.commit_sha,
            update_id: context.update_id,
            previous_revision_id: context.previous_revision_id,
            previous_release_id: context.previous_release_id,
            previous_parameters: context.previous_parameters,
            artifacts,
            previous_artifacts,
        })
    }

    async fn run_is_live(&self, run_id: RunId) -> Result<bool, RunRuntimeCatalogError> {
        sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM runs
                WHERE id = $1 AND state <> 'cleaned_up'
             )",
        )
        .bind(run_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(storage)
    }
}

impl PgRunRepository {
    async fn load_runtime_artifacts(
        &self,
        release_id: Uuid,
    ) -> Result<Vec<RunRuntimeArtifact>, RunRuntimeCatalogError> {
        sqlx::query_as::<_, RuntimeArtifactRow>(
            "SELECT path, kind, mode, content_hash, size_bytes, storage_key
             FROM release_artifacts
             WHERE release_id = $1
               AND kind IN ('executable', 'file', 'manifest')
             ORDER BY path, id",
        )
        .bind(release_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }
}

#[derive(Debug, FromRow)]
struct RuntimeContextRow {
    parameters: Value,
    repository_id: Option<Uuid>,
    git_ref: Option<String>,
    commit_sha: Option<String>,
    release_state: String,
    update_id: Option<Uuid>,
    previous_revision_id: Option<Uuid>,
    previous_release_id: Option<Uuid>,
    previous_parameters: Option<Value>,
}

#[derive(Debug, FromRow)]
struct RuntimeArtifactRow {
    path: String,
    kind: String,
    mode: i32,
    content_hash: Vec<u8>,
    size_bytes: i64,
    storage_key: Uuid,
}

impl TryFrom<RuntimeArtifactRow> for RunRuntimeArtifact {
    type Error = RunRuntimeCatalogError;

    fn try_from(row: RuntimeArtifactRow) -> Result<Self, Self::Error> {
        let kind = match row.kind.as_str() {
            "executable" => RunRuntimeArtifactKind::Executable,
            "file" => RunRuntimeArtifactKind::File,
            "manifest" => RunRuntimeArtifactKind::Manifest,
            _ => return Err(RunRuntimeCatalogError::InvalidData("artifact kind")),
        };
        let content_hash = row
            .content_hash
            .try_into()
            .map_err(|_| RunRuntimeCatalogError::InvalidData("artifact content hash"))?;
        let mode = u32::try_from(row.mode)
            .map_err(|_| RunRuntimeCatalogError::InvalidData("artifact mode"))?;
        let size_bytes = u64::try_from(row.size_bytes)
            .map_err(|_| RunRuntimeCatalogError::InvalidData("artifact size"))?;
        Ok(Self {
            path: row.path,
            kind,
            mode,
            content_hash,
            size_bytes,
            storage_key: row.storage_key,
        })
    }
}

const fn run_kind_name(kind: RunKind) -> &'static str {
    match kind {
        RunKind::Normal => "normal",
        RunKind::Update => "update",
    }
}

fn storage(error: sqlx::Error) -> RunRuntimeCatalogError {
    RunRuntimeCatalogError::Storage(Box::new(error))
}
