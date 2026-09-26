use super::models::BuildSourceRow;
use super::models::parse_state;
use super::ui_manifest;
use super::{
    BUILD_REQUESTED_SUBJECT, BuildApplication, BuildError, RequestBuild, RequestedBuild,
    encode_hash,
};
use agent_config::AgentConfig;
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use serde_json::{Value, json};
use sqlx::{FromRow, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, FromRow)]
struct ResolvedImageRow {
    catalog_image_id: Option<Uuid>,
    repository_image_id: Option<Uuid>,
    key: String,
    reference: String,
}

impl BuildApplication {
    // The transaction intentionally keeps validation, durable request creation,
    // source linkage, and its outbox record visibly atomic.
    #[allow(clippy::too_many_lines)]
    pub async fn request_build(
        &self,
        identity: &AuthenticatedIdentity,
        request: RequestBuild,
    ) -> Result<RequestedBuild, BuildError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(BuildError::Persistence)?;
        let repository_visible: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM repositories WHERE id = $1 FOR NO KEY UPDATE")
                .bind(request.repository_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(BuildError::Persistence)?;
        if repository_visible.is_none() {
            return Err(BuildError::FailedPrecondition);
        }
        let configuration_hash = encode_hash(&request.configuration_hash);
        let source_commit = forge_domain::CommitSha::parse(&request.source_commit)
            .map_err(|_| BuildError::FailedPrecondition)?;
        let snapshot = ui_manifest::lookup_ui_manifest_snapshot(
            &mut transaction,
            forge_domain::RepositoryId::from_uuid(request.repository_id),
            &source_commit,
        )
        .await?;
        let source = sqlx::query_as::<_, BuildSourceRow>(
            "SELECT revision.receive_id, revision.config, reference.git_ref
             FROM agent_config_revisions revision
             JOIN git_refs reference
               ON reference.repository_id = revision.repository_id
              AND reference.commit_sha = revision.commit_sha
             WHERE revision.repository_id = $1
               AND revision.commit_sha = $2
               AND revision.normalized_config_hash = $3
               AND revision.status = 'valid'
               AND revision.config IS NOT NULL
             ORDER BY reference.git_ref, revision.created_at DESC, revision.id
             LIMIT 1",
        )
        .bind(request.repository_id)
        .bind(&request.source_commit)
        .bind(&configuration_hash)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?
        .ok_or(BuildError::FailedPrecondition)?;
        let config: AgentConfig =
            serde_json::from_value(source.config).map_err(BuildError::Serialization)?;
        let build = config
            .build
            .as_ref()
            .ok_or(BuildError::FailedPrecondition)?;
        let base_build_hash = agent_config::build_identity::base_build_definition_hash(build)
            .map_err(BuildError::Serialization)?;
        let build_definition_hash = ui_manifest::resolve_build_definition_hash(
            snapshot,
            base_build_hash,
            request.build_definition_hash,
        )?;
        let resolved_build_image =
            resolve_image(&mut transaction, request.repository_id, &build.image).await?;
        let resolved_guest_image =
            resolve_image(&mut transaction, request.repository_id, &config.guest.image).await?;
        let build_declaration = serde_json::to_value(build).map_err(BuildError::Serialization)?;
        let build_policy = json!({
            "resources": build.resources,
            "network": build.network,
        });
        let declared_artifacts =
            serde_json::to_value(&build.artifacts).map_err(BuildError::Serialization)?;
        let agent_key = config.agent.key.as_deref();

        let now = OffsetDateTime::now_utc();
        let existing: Option<(Uuid, OffsetDateTime, String, OffsetDateTime)> = sqlx::query_as(
            "SELECT id, created_at, state,
                    COALESCE(completed_at, started_at, created_at)
             FROM build_requests
             WHERE repository_id = $1 AND source_commit = $2
               AND source_ref = $3 AND build_definition_hash = $4",
        )
        .bind(request.repository_id)
        .bind(&request.source_commit)
        .bind(&source.git_ref)
        .bind(build_definition_hash.as_slice())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?;
        let (row, is_new) = if let Some(row) = existing {
            (row, false)
        } else {
            let requested_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO build_requests
                 (id, repository_id, source_commit, source_ref, origin_receive_id,
                  build_definition_hash, state, created_by, created_at, build_trigger,
                  agent_key,
                  configuration_hash, build_declaration, build_policy, declared_artifacts)
                 VALUES ($1, $2, $3, $4, $5, $6, 'queued', $7, $8, 'manual', $9,
                         decode($10, 'hex'), $11, $12, $13)",
            )
            .bind(requested_id)
            .bind(request.repository_id)
            .bind(&request.source_commit)
            .bind(&source.git_ref)
            .bind(source.receive_id)
            .bind(build_definition_hash.as_slice())
            .bind(identity.user_id.as_uuid())
            .bind(now)
            .bind(agent_key)
            .bind(&configuration_hash)
            .bind(build_declaration)
            .bind(build_policy)
            .bind(declared_artifacts)
            .execute(&mut *transaction)
            .await
            .map_err(BuildError::Persistence)?;
            let row: (Uuid, OffsetDateTime, String, OffsetDateTime) = sqlx::query_as(
                "SELECT id, created_at, state,
                        COALESCE(completed_at, started_at, created_at)
                 FROM build_requests
                 WHERE repository_id = $1 AND source_commit = $2
                   AND source_ref = $3 AND build_definition_hash = $4",
            )
            .bind(request.repository_id)
            .bind(&request.source_commit)
            .bind(&source.git_ref)
            .bind(build_definition_hash.as_slice())
            .fetch_one(&mut *transaction)
            .await
            .map_err(BuildError::Persistence)?;
            (row, true)
        };
        sqlx::query(
            "INSERT INTO build_request_images
             (build_request_id, execution_context, image_id, repository_oci_image_id,
              image_key, image_reference)
             VALUES ($1, 'build', $2, $3, $4, $5),
                    ($1, 'guest', $6, $7, $8, $9)
             ON CONFLICT (build_request_id, execution_context) DO NOTHING",
        )
        .bind(row.0)
        .bind(resolved_build_image.catalog_image_id)
        .bind(resolved_build_image.repository_image_id)
        .bind(&resolved_build_image.key)
        .bind(&resolved_build_image.reference)
        .bind(resolved_guest_image.catalog_image_id)
        .bind(resolved_guest_image.repository_image_id)
        .bind(&resolved_guest_image.key)
        .bind(&resolved_guest_image.reference)
        .execute(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?;
        sqlx::query(
            "INSERT INTO build_request_sources
             (build_request_id, receive_id, source_ref, source_commit, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT DO NOTHING",
        )
        .bind(row.0)
        .bind(source.receive_id)
        .bind(&source.git_ref)
        .bind(&request.source_commit)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?;
        if let ui_manifest::UiManifestSnapshot::Valid { .. } = snapshot {
            ui_manifest::link_ui_manifest_to_build(
                &mut transaction,
                release_domain::BuildRequestId::from_uuid(row.0),
                forge_domain::RepositoryId::from_uuid(request.repository_id),
                &source_commit,
                snapshot,
            )
            .await?;
        }
        if !is_new {
            // A deduplicated request does not update build_requests, so its
            // row trigger cannot create the read-your-writes receipt required
            // by this fresh mutation occurrence. Reuse the durable event
            // function in this transaction without enqueueing another build.
            let receipt_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                     SELECT 1
                     FROM application_events
                     WHERE occurrence_id = $1
                       AND aggregate_type = 'build'
                       AND aggregate_id = $2
                       AND scope_kind = 'repository'
                       AND scope_id = $3
                       AND actor_id = $4
                 )",
            )
            .bind(identity.idempotency_id.as_uuid())
            .bind(row.0)
            .bind(request.repository_id)
            .bind(identity.user_id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(BuildError::Persistence)?;
            if !receipt_exists {
                sqlx::query(
                    "SELECT event_id
                     FROM append_application_event(
                         $1, 'repository', $2, 'build', $3,
                         'build.changed', 'updated', $4, $2, NULL
                     )",
                )
                .bind(identity.idempotency_id.as_uuid())
                .bind(request.repository_id)
                .bind(row.0)
                .bind(&row.2)
                .fetch_one(&mut *transaction)
                .await
                .map_err(BuildError::Persistence)?;
            }
        }
        if is_new {
            let event_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO outbox
                 (id, aggregate_type, aggregate_id, subject, event_type, payload,
                  occurred_at)
                 VALUES ($1, 'forge', $2, $3, 'build.requested.v1', $4, $5)",
            )
            .bind(event_id)
            .bind(row.0)
            .bind(BUILD_REQUESTED_SUBJECT)
            .bind(json!({
                "schema_version": 1,
                "message_id": event_id,
                "idempotency_key": event_id,
                "request_id": identity.request_id,
                "trace_id": Value::Null,
                "build_request_id": row.0,
                "repository_id": request.repository_id,
                "source_commit": request.source_commit,
                "source_ref": source.git_ref,
                "receive_id": source.receive_id,
                "normalized_configuration_hash": configuration_hash,
                "build_definition_hash": encode_hash(&build_definition_hash),
            }))
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(BuildError::Persistence)?;
        }
        transaction
            .commit()
            .await
            .map_err(BuildError::Persistence)?;
        Ok(RequestedBuild {
            id: row.0,
            state: parse_state(&row.2)?,
            created_at: row.1,
            updated_at: row.3,
        })
    }
}

async fn resolve_image(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    selection: &agent_config::ImageSelection,
) -> Result<ResolvedImageRow, BuildError> {
    let row = match (&selection.key, &selection.project_image) {
        (Some(key), None) => sqlx::query_as::<_, ResolvedImageRow>(
            "SELECT id AS catalog_image_id, NULL::uuid AS repository_image_id,
                    key, image_reference AS reference
               FROM oci_images
              WHERE key = $1 AND availability_state = 'available' AND role = 'execution'",
        )
        .bind(key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(BuildError::Persistence)?,
        (None, Some(key)) => sqlx::query_as::<_, ResolvedImageRow>(
            "SELECT NULL::uuid AS catalog_image_id, image.id AS repository_image_id,
                    image.key, image.image_reference AS reference
               FROM repository_oci_image_definitions AS image
               JOIN repositories AS repository ON repository.id = $1
              WHERE image.project_id = repository.project_id
                AND image.key = $2
                AND image.status = 'ready'
              ORDER BY image.updated_at DESC, image.id DESC
              LIMIT 1",
        )
        .bind(repository_id)
        .bind(key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(BuildError::Persistence)?,
        _ => None,
    };
    let row = row.ok_or(BuildError::FailedPrecondition)?;
    Ok(row)
}
