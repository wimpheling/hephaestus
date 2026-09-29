use super::models::BuildRow;
use super::{BuildApplication, BuildError, BuildPage, BuildPageResult, BuildView};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use uuid::Uuid;

impl BuildApplication {
    pub async fn get_build(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<BuildView, BuildError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(BuildError::Persistence)?;
        let row = sqlx::query_as::<_, BuildRow>(
            "SELECT request.id, request.repository_id, request.state, execution.exit_code,
                    execution.failure_code, execution.logs, execution.metrics,
                    request.created_at,
                    COALESCE(execution.updated_at, request.completed_at,
                             request.started_at, request.created_at) AS updated_at,
                    request.source_commit, request.source_ref,
                    encode(request.build_definition_hash, 'hex') AS build_definition_hash,
                    request.build_trigger, request.agent_key,
                    (SELECT image_id FROM build_request_images
                      WHERE build_request_id = request.id
                        AND execution_context = 'build') AS image_id,
                    (SELECT image_key FROM build_request_images
                      WHERE build_request_id = request.id
                        AND execution_context = 'build') AS image_key,
                    (SELECT image_reference FROM build_request_images
                      WHERE build_request_id = request.id
                        AND execution_context = 'build') AS image_reference,
                    encode(request.configuration_hash, 'hex') AS configuration_hash,
                    request.build_declaration, request.build_policy,
                    request.declared_artifacts, execution.started_at,
                    COALESCE(execution.completed_at, request.completed_at) AS completed_at,
                    execution.artifact_manifest,
                    release.id AS release_id, release.state AS release_state,
                    release.version AS release_version,
                    (SELECT count(*) FROM release_artifacts artifact
                     WHERE artifact.release_id = release.id)::bigint AS artifact_count
                    ,COALESCE((SELECT jsonb_agg(
                        jsonb_build_object(
                            'from_state', transition.from_state,
                            'to_state', transition.to_state,
                            'reason', transition.reason,
                            'occurred_at', transition.occurred_at
                        ) ORDER BY transition.occurred_at, transition.id
                    ) FROM build_state_transitions transition
                    WHERE transition.build_request_id = request.id), '[]'::jsonb) AS timeline
                    ,COALESCE((SELECT jsonb_agg(
                        jsonb_build_object(
                            'path', artifact.path,
                            'kind', artifact.kind,
                            'mode', artifact.mode,
                            'sha256', encode(artifact.content_hash, 'hex'),
                            'size_bytes', artifact.size_bytes,
                            'media_type', artifact.media_type
                        ) ORDER BY artifact.path, artifact.id
                    ) FROM release_artifacts artifact
                    WHERE artifact.release_id = release.id), '[]'::jsonb) AS produced_artifacts
                    ,COALESCE((SELECT jsonb_agg(
                        jsonb_build_object(
                            'state', verification.state,
                            'expected_manifest', verification.expected_manifest,
                            'actual_manifest', verification.actual_manifest,
                            'failure_code', verification.failure_code,
                            'created_at', verification.created_at,
                            'completed_at', verification.completed_at
                        ) ORDER BY verification.created_at DESC, verification.id DESC
                    ) FROM build_verifications verification
                    WHERE verification.build_request_id = request.id), '[]'::jsonb) AS verifications
             FROM build_requests request
             LEFT JOIN build_executions execution
               ON execution.build_request_id = request.id
             LEFT JOIN releases release ON release.build_request_id = request.id
             WHERE request.id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?
        .ok_or(BuildError::NotFound)?;
        transaction
            .commit()
            .await
            .map_err(BuildError::Persistence)?;
        row.try_into()
    }

    pub async fn list_builds(
        &self,
        identity: &AuthenticatedIdentity,
        repository_id: Uuid,
        page: BuildPage,
    ) -> Result<BuildPageResult, BuildError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(BuildError::Persistence)?;
        let rows = sqlx::query_as::<_, BuildRow>(
            "SELECT request.id, request.repository_id, request.state, execution.exit_code,
                    execution.failure_code, execution.logs, execution.metrics,
                    request.created_at,
                    COALESCE(execution.updated_at, request.completed_at,
                             request.started_at, request.created_at) AS updated_at,
                    request.source_commit, request.source_ref,
                    encode(request.build_definition_hash, 'hex') AS build_definition_hash,
                    request.build_trigger, request.agent_key,
                    (SELECT image_id FROM build_request_images
                      WHERE build_request_id = request.id
                        AND execution_context = 'build') AS image_id,
                    (SELECT image_key FROM build_request_images
                      WHERE build_request_id = request.id
                        AND execution_context = 'build') AS image_key,
                    (SELECT image_reference FROM build_request_images
                      WHERE build_request_id = request.id
                        AND execution_context = 'build') AS image_reference,
                    encode(request.configuration_hash, 'hex') AS configuration_hash,
                    request.build_declaration, request.build_policy,
                    request.declared_artifacts, execution.started_at,
                    COALESCE(execution.completed_at, request.completed_at) AS completed_at,
                    execution.artifact_manifest,
                    release.id AS release_id, release.state AS release_state,
                    release.version AS release_version,
                    (SELECT count(*) FROM release_artifacts artifact
                     WHERE artifact.release_id = release.id)::bigint AS artifact_count
                    ,COALESCE((SELECT jsonb_agg(
                        jsonb_build_object(
                            'from_state', transition.from_state,
                            'to_state', transition.to_state,
                            'reason', transition.reason,
                            'occurred_at', transition.occurred_at
                        ) ORDER BY transition.occurred_at, transition.id
                    ) FROM build_state_transitions transition
                    WHERE transition.build_request_id = request.id), '[]'::jsonb) AS timeline
                    ,COALESCE((SELECT jsonb_agg(
                        jsonb_build_object(
                            'path', artifact.path,
                            'kind', artifact.kind,
                            'mode', artifact.mode,
                            'sha256', encode(artifact.content_hash, 'hex'),
                            'size_bytes', artifact.size_bytes,
                            'media_type', artifact.media_type
                        ) ORDER BY artifact.path, artifact.id
                    ) FROM release_artifacts artifact
                    WHERE artifact.release_id = release.id), '[]'::jsonb) AS produced_artifacts
                    ,COALESCE((SELECT jsonb_agg(
                        jsonb_build_object(
                            'state', verification.state,
                            'expected_manifest', verification.expected_manifest,
                            'actual_manifest', verification.actual_manifest,
                            'failure_code', verification.failure_code,
                            'created_at', verification.created_at,
                            'completed_at', verification.completed_at
                        ) ORDER BY verification.created_at DESC, verification.id DESC
                    ) FROM build_verifications verification
                    WHERE verification.build_request_id = request.id), '[]'::jsonb) AS verifications
             FROM build_requests request
             LEFT JOIN build_executions execution
               ON execution.build_request_id = request.id
             LEFT JOIN releases release ON release.build_request_id = request.id
             WHERE request.repository_id = $1
               AND ($2::uuid IS NULL OR (request.created_at, request.id) <
                    (SELECT cursor.created_at, cursor.id
                     FROM build_requests cursor WHERE cursor.id = $2))
             ORDER BY request.created_at DESC, request.id DESC
             LIMIT $3",
        )
        .bind(repository_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(BuildError::Persistence)?;
        let size = usize::try_from(page.size).map_err(|_| BuildError::InvalidStoredData)?;
        let has_more = rows.len() > size;
        let builds: Vec<BuildView> = rows
            .into_iter()
            .take(size)
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, BuildError>>()?;
        let next = has_more
            .then(|| builds.last().map(|build| build.id))
            .flatten();
        Ok(BuildPageResult { builds, next })
    }
}
