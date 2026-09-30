use super::{
    PgBuildRepository,
    model::{InputRow, claimed, manifest_projection, storage},
};
use build_orchestrator::{BuildRepositoryError, ClaimedBuild};
use release_domain::{BuildRequestId, ReleaseAgentId, ReleaseId, ReleaseVersion};
use serde_json::Value;
use uuid::Uuid;

impl PgBuildRepository {
    /// Runs an internal build persistence operation.
    ///
    /// # Errors
    ///
    /// Returns a storage or provider-state error when persistence fails.
    pub async fn image_reference_impl(
        &self,
        id: BuildRequestId,
    ) -> Result<String, BuildRepositoryError> {
        sqlx::query_scalar(
            "SELECT image_reference
               FROM build_request_images
              WHERE build_request_id = $1
                AND execution_context = 'build'",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(BuildRepositoryError::Unavailable)
    }

    /// Runs an internal build persistence operation.
    ///
    /// # Errors
    ///
    /// Returns a storage or provider-state error when persistence fails.
    pub async fn reset_for_retry_impl(
        &self,
        id: BuildRequestId,
    ) -> Result<(), BuildRepositoryError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let changed = sqlx::query(
            "INSERT INTO build_attempts
                 (id, build_request_id, attempt_number, state, failure_code,
                  artifact_manifest, started_at, completed_at)
             SELECT gen_random_uuid(), build_request_id, attempt_number, state,
                    failure_code, artifact_manifest, started_at, completed_at
               FROM build_executions
              WHERE build_request_id = $1
                AND state IN ('failed', 'claimed')
             ON CONFLICT (build_request_id, attempt_number) DO NOTHING",
        )
        .bind(id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        if changed.rows_affected() == 0 {
            return Err(BuildRepositoryError::AlreadyClaimed);
        }
        sqlx::query(
            "UPDATE build_executions
                SET attempt_number = attempt_number + 1,
                    state = 'claimed', failure_code = NULL,
                    exit_code = NULL, exit_signal = NULL,
                    logs = '[]'::jsonb, metrics = '[]'::jsonb,
                    artifact_manifest = NULL, started_at = NULL,
                    sealed_at = NULL, imported_at = NULL,
                    completed_at = NULL, updated_at = now()
              WHERE build_request_id = $1 AND state IN ('failed', 'claimed')",
        )
        .bind(id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)
    }

    /// Runs an internal build persistence operation.
    ///
    /// # Errors
    ///
    /// Returns a storage or provider-state error when persistence fails.
    pub async fn claim_verification_impl(
        &self,
        id: BuildRequestId,
    ) -> Result<ClaimedBuild, BuildRepositoryError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let input: InputRow = sqlx::query_as(
            "SELECT request.repository_id, request.source_commit, request.source_ref,
                    selected.image_reference, request.state, revision.config,
                    request.created_by
             FROM build_requests AS request
             JOIN build_request_images AS selected
               ON selected.build_request_id = request.id
              AND selected.execution_context = 'build'
               JOIN LATERAL (
                   SELECT config FROM agent_config_revisions
                    WHERE repository_id = request.repository_id
                      AND commit_sha = request.source_commit
                      AND status = 'valid'
                    ORDER BY created_at DESC, id
                    LIMIT 1
               ) AS revision ON true
              WHERE request.id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?
        .ok_or(BuildRepositoryError::Unavailable)?;
        let (release, agent, version): (Uuid, Uuid, String) = sqlx::query_as(
            "SELECT release_id, release_agent_id, release_version
               FROM build_executions
              WHERE build_request_id = $1 AND state = 'drafted'",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?
        .ok_or(BuildRepositoryError::Unavailable)?;
        let inserted = sqlx::query(
            "INSERT INTO build_verifications
                 (id, build_request_id, state, expected_manifest)
             SELECT gen_random_uuid(), $1, 'running',
                    COALESCE(
                        jsonb_agg(jsonb_build_object(
                            'path', path,
                            'kind', kind,
                            'mode', mode,
                            'content_hash', encode(content_hash, 'hex'),
                            'size_bytes', size_bytes,
                            'media_type', media_type
                        ) ORDER BY path), '[]'::jsonb)
               FROM release_artifacts
              WHERE release_id = $2
                AND NOT EXISTS (
                    SELECT 1 FROM build_verifications
                     WHERE build_request_id = $1 AND state = 'running'
                )",
        )
        .bind(id.as_uuid())
        .bind(release)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        if inserted.rows_affected() == 0 {
            return Err(BuildRepositoryError::AlreadyClaimed);
        }
        tx.commit().await.map_err(storage)?;
        claimed(
            id,
            input,
            ReleaseId::from_uuid(release),
            ReleaseAgentId::from_uuid(agent),
            ReleaseVersion::parse(version).map_err(|_| BuildRepositoryError::InvalidData)?,
        )
    }

    /// Runs an internal build persistence operation.
    ///
    /// # Errors
    ///
    /// Returns a storage or provider-state error when persistence fails.
    pub async fn complete_verification_impl(
        &self,
        id: BuildRequestId,
        actual_manifest: &Value,
    ) -> Result<bool, BuildRepositoryError> {
        let expected: Value = sqlx::query_scalar(
            "SELECT expected_manifest
               FROM build_verifications
              WHERE build_request_id = $1 AND state = 'running'
              ORDER BY created_at DESC, id DESC
              LIMIT 1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(BuildRepositoryError::Unavailable)?;
        let matches = manifest_projection(&expected) == manifest_projection(actual_manifest);
        sqlx::query(
            "UPDATE build_verifications
                SET state = $2,
                    actual_manifest = $3,
                    failure_code = CASE WHEN $4 THEN NULL ELSE 'manifest_mismatch' END,
                    completed_at = now()
              WHERE build_request_id = $1 AND state = 'running'",
        )
        .bind(id.as_uuid())
        .bind(if matches { "succeeded" } else { "failed" })
        .bind(actual_manifest)
        .bind(matches)
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(matches)
    }

    /// Runs an internal build persistence operation.
    ///
    /// # Errors
    ///
    /// Returns a storage or provider-state error when persistence fails.
    pub async fn fail_verification_impl(
        &self,
        id: BuildRequestId,
        code: &str,
    ) -> Result<(), BuildRepositoryError> {
        sqlx::query(
            "UPDATE build_verifications
                SET state = 'failed', failure_code = $2, completed_at = now()
              WHERE build_request_id = $1 AND state = 'running'",
        )
        .bind(id.as_uuid())
        .bind(code)
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }
}
