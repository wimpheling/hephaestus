use super::{
    PgBuildRepository,
    model::{InputRow, claimed, storage},
};
use build_orchestrator::{BuildRepositoryError, FinalizationBuild, RecoverableBuild};
use release_domain::{BuildRequestId, ReleaseAgentId, ReleaseId, ReleaseVersion};
use serde_json::Value;
use uuid::Uuid;

impl PgBuildRepository {
    /// Runs an internal build persistence operation.
    ///
    /// # Errors
    ///
    /// Returns a storage or provider-state error when persistence fails.
    pub async fn recoverable_impl(&self) -> Result<Vec<RecoverableBuild>, BuildRepositoryError> {
        let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT build_request_id, vm_id FROM build_executions WHERE state IN ('claimed','running') ORDER BY updated_at, build_request_id")
            .fetch_all(&self.pool).await.map_err(storage)?;
        Ok(rows
            .into_iter()
            .map(|(id, vm_id)| RecoverableBuild {
                id: BuildRequestId::from_uuid(id),
                vm_id,
            })
            .collect())
    }

    /// Runs an internal build persistence operation.
    ///
    /// # Errors
    ///
    /// Returns a storage or provider-state error when persistence fails.
    pub async fn finalizing_impl(&self) -> Result<Vec<BuildRequestId>, BuildRepositoryError> {
        let rows: Vec<Uuid> = sqlx::query_scalar("SELECT build_request_id FROM build_executions WHERE state IN ('sealed','imported') ORDER BY updated_at, build_request_id")
            .fetch_all(&self.pool).await.map_err(storage)?;
        Ok(rows.into_iter().map(BuildRequestId::from_uuid).collect())
    }

    /// Runs an internal build persistence operation.
    ///
    /// # Errors
    ///
    /// Returns a storage or provider-state error when persistence fails.
    pub async fn reset_after_cleanup_impl(
        &self,
        id: BuildRequestId,
    ) -> Result<(), BuildRepositoryError> {
        sqlx::query("UPDATE build_executions SET state='claimed', exit_code=NULL, exit_signal=NULL, logs='[]', metrics='[]', started_at=NULL, updated_at=now() WHERE build_request_id=$1 AND state IN ('claimed','running')")
            .bind(id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    /// Runs an internal build persistence operation.
    ///
    /// # Errors
    ///
    /// Returns a storage or provider-state error when persistence fails.
    pub async fn completed_impl(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<(ReleaseId, ReleaseAgentId, ReleaseVersion, usize)>, BuildRepositoryError>
    {
        let row: Option<(Uuid, Uuid, String, Value)> = sqlx::query_as("SELECT release_id, release_agent_id, release_version, artifact_manifest FROM build_executions WHERE build_request_id = $1 AND state = 'drafted'")
            .bind(id.as_uuid()).fetch_optional(&self.pool).await.map_err(storage)?;
        row.map(|(release, agent, version, manifest)| {
            let count = manifest
                .as_array()
                .ok_or(BuildRepositoryError::InvalidData)?
                .len();
            Ok((
                ReleaseId::from_uuid(release),
                ReleaseAgentId::from_uuid(agent),
                ReleaseVersion::parse(version).map_err(|_| BuildRepositoryError::InvalidData)?,
                count,
            ))
        })
        .transpose()
    }

    /// Runs an internal build persistence operation.
    ///
    /// # Errors
    ///
    /// Returns a storage or provider-state error when persistence fails.
    pub async fn finalization_impl(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<FinalizationBuild>, BuildRepositoryError> {
        let row: Option<(String, Uuid, Uuid, String, Option<Value>)> = sqlx::query_as("SELECT state, release_id, release_agent_id, release_version, artifact_manifest FROM build_executions WHERE build_request_id = $1 AND state IN ('sealed','imported')")
            .bind(id.as_uuid()).fetch_optional(&self.pool).await.map_err(storage)?;
        let Some((state, release, agent, version, artifact_manifest)) = row else {
            return Ok(None);
        };
        let input: InputRow = sqlx::query_as("SELECT request.repository_id, request.source_commit, request.source_ref, selected.image_reference, request.state, revision.config, request.created_by FROM build_requests AS request JOIN build_request_images AS selected ON selected.build_request_id = request.id AND selected.execution_context = 'build' JOIN LATERAL (SELECT config FROM agent_config_revisions WHERE repository_id = request.repository_id AND commit_sha = request.source_commit AND status = 'valid' ORDER BY created_at DESC LIMIT 1) AS revision ON true WHERE request.id = $1")
            .bind(id.as_uuid()).fetch_optional(&self.pool).await.map_err(storage)?.ok_or(BuildRepositoryError::Unavailable)?;
        Ok(Some(FinalizationBuild {
            state,
            claimed: claimed(
                id,
                input,
                ReleaseId::from_uuid(release),
                ReleaseAgentId::from_uuid(agent),
                ReleaseVersion::parse(version).map_err(|_| BuildRepositoryError::InvalidData)?,
            )?,
            artifact_manifest,
        }))
    }
}
