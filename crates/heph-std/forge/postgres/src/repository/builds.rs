use super::{
    helpers::storage,
    images::hex_digest,
    images::{ResolvedImageRow, resolve_image},
    outbox::append_outbox,
};
use crate::BUILD_REQUESTED_SUBJECT;
use agent_config::ConfigHash;
use forge_domain::{CommitSha, GitRef, ReceiveId, RepositoryId};
use forge_service::ForgeRepositoryError;
use identity_domain::AuthenticatedIdentity;
use release_domain::BuildRequestId;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

pub fn build_trigger_matches(patterns: &[String], git_ref: &GitRef) -> bool {
    patterns.iter().any(|pattern| {
        pattern.strip_suffix("/*").map_or_else(
            || pattern == git_ref.as_str(),
            |prefix| {
                git_ref
                    .as_str()
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            },
        )
    })
}

// This parameter set carries the complete immutable build request identity and transaction context.
#[allow(clippy::too_many_arguments)]
pub async fn persist_build_request(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    receive_id: ReceiveId,
    git_ref: &GitRef,
    commit: &CommitSha,
    build: &agent_config::BuildConfig,
    guest_image: &agent_config::ImageSelection,
    agent_key: Option<&str>,
    normalized_hash: &ConfigHash,
    build_definition_hash: [u8; 32],
    identity: Option<&AuthenticatedIdentity>,
    now: OffsetDateTime,
) -> Result<BuildRequestId, ForgeRepositoryError> {
    let build_declaration =
        serde_json::to_value(build).map_err(ForgeRepositoryError::Serialization)?;
    let build_policy = json!({
        "resources": build.resources,
        "network": build.network,
    });
    let declared_artifacts =
        serde_json::to_value(&build.artifacts).map_err(ForgeRepositoryError::Serialization)?;
    let build_image = resolve_image(transaction, repository_id, &build.image).await?;
    let guest_image = resolve_image(transaction, repository_id, guest_image).await?;
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id
         FROM build_requests
         WHERE repository_id = $1 AND source_commit = $2
           AND source_ref = $3 AND build_definition_hash = $4",
    )
    .bind(repository_id.as_uuid())
    .bind(commit.as_str())
    .bind(git_ref.as_str())
    .bind(build_definition_hash.as_slice())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?;
    let (stored_id, is_new) = if let Some(existing) = existing {
        (existing, false)
    } else {
        let requested_id = BuildRequestId::new();
        sqlx::query(
            "INSERT INTO build_requests
             (id, repository_id, source_commit, source_ref, origin_receive_id,
              build_definition_hash, state, created_by, created_at, build_trigger,
              agent_key, configuration_hash, build_declaration, build_policy,
              declared_artifacts)
             VALUES ($1, $2, $3, $4, $5, $6, 'queued', $7, $8, 'push', $9,
                     decode($10, 'hex'), $11, $12, $13)",
        )
        .bind(requested_id.as_uuid())
        .bind(repository_id.as_uuid())
        .bind(commit.as_str())
        .bind(git_ref.as_str())
        .bind(receive_id.as_uuid())
        .bind(build_definition_hash.as_slice())
        .bind(identity.map(|value| value.user_id.as_uuid()))
        .bind(now)
        .bind(agent_key)
        .bind(normalized_hash.as_str())
        .bind(build_declaration)
        .bind(build_policy)
        .bind(declared_artifacts)
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
        let stored_id: Uuid = sqlx::query_scalar(
            "SELECT id
             FROM build_requests
             WHERE repository_id = $1 AND source_commit = $2
               AND source_ref = $3 AND build_definition_hash = $4",
        )
        .bind(repository_id.as_uuid())
        .bind(commit.as_str())
        .bind(git_ref.as_str())
        .bind(build_definition_hash.as_slice())
        .fetch_one(&mut **transaction)
        .await
        .map_err(storage)?;
        (stored_id, true)
    };
    persist_build_request_images(transaction, stored_id, &build_image, &guest_image).await?;
    persist_build_request_source(transaction, stored_id, receive_id, git_ref, commit, now).await?;
    if is_new {
        append_outbox(
            transaction,
            stored_id,
            BUILD_REQUESTED_SUBJECT,
            "build.requested.v1",
            json!({
                "schema_version": 1,
                "build_request_id": stored_id,
                "repository_id": repository_id,
                "source_commit": commit,
                "source_ref": git_ref,
                "receive_id": receive_id,
                "normalized_configuration_hash": normalized_hash,
                "build_definition_hash": hex_digest(&build_definition_hash),
            }),
            now,
        )
        .await?;
    }
    Ok(BuildRequestId::from_uuid(stored_id))
}

async fn persist_build_request_images(
    transaction: &mut Transaction<'_, Postgres>,
    build_request_id: Uuid,
    build_image: &ResolvedImageRow,
    guest_image: &ResolvedImageRow,
) -> Result<(), ForgeRepositoryError> {
    for (execution_context, image) in [("build", build_image), ("guest", guest_image)] {
        sqlx::query(
            "INSERT INTO build_request_images
                (build_request_id, execution_context, image_id, repository_oci_image_id,
                 image_key, image_reference)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT DO NOTHING",
        )
        .bind(build_request_id)
        .bind(execution_context)
        .bind(image.catalog_image_id)
        .bind(image.repository_image_id)
        .bind(&image.key)
        .bind(&image.image_reference)
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
    }
    Ok(())
}

async fn persist_build_request_source(
    transaction: &mut Transaction<'_, Postgres>,
    build_request_id: Uuid,
    receive_id: ReceiveId,
    git_ref: &GitRef,
    commit: &CommitSha,
    now: OffsetDateTime,
) -> Result<(), ForgeRepositoryError> {
    sqlx::query(
        "INSERT INTO build_request_sources
         (build_request_id, receive_id, source_ref, source_commit, created_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING",
    )
    .bind(build_request_id)
    .bind(receive_id.as_uuid())
    .bind(git_ref.as_str())
    .bind(commit.as_str())
    .bind(now)
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok(())
}
