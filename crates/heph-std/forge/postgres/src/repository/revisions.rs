use super::{
    helpers::{serialization, storage},
    inspect::InspectedRepositoryOciImage,
};
use agent_config::{ConfigHash, ParsedConfig};
use forge_domain::{AgentConfigRevisionId, CommitSha, ProjectId, ReceiveId, RepositoryId};
use forge_service::ForgeRepositoryError;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

pub async fn persist_revision(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    receive_id: ReceiveId,
    commit: &CommitSha,
    parsed: &ParsedConfig,
    now: OffsetDateTime,
) -> Result<AgentConfigRevisionId, ForgeRepositoryError> {
    let status = if parsed.config.is_some() {
        "valid"
    } else {
        "invalid"
    };
    let config = parsed
        .config
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(serialization)?;
    let diagnostics = serde_json::to_value(&parsed.diagnostics).map_err(serialization)?;
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id
         FROM agent_config_revisions
         WHERE repository_id = $1 AND commit_sha = $2 AND config_hash = $3",
    )
    .bind(repository_id.as_uuid())
    .bind(commit.as_str())
    .bind(parsed.hash.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?;
    let stored_id = if let Some(existing) = existing {
        existing
    } else {
        let revision_id = AgentConfigRevisionId::new();
        sqlx::query(
            "INSERT INTO agent_config_revisions
             (id, repository_id, receive_id, commit_sha, config_hash, schema_version,
              status, config, diagnostics, created_at,
              normalized_config_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(revision_id.as_uuid())
        .bind(repository_id.as_uuid())
        .bind(receive_id.as_uuid())
        .bind(commit.as_str())
        .bind(parsed.hash.as_str())
        .bind(
            parsed
                .config
                .as_ref()
                .map(|config| i32::try_from(config.version).unwrap_or(i32::MAX)),
        )
        .bind(status)
        .bind(config)
        .bind(diagnostics)
        .bind(now)
        .bind(parsed.normalized_hash.as_ref().map(ConfigHash::as_str))
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
        let stored_id: Uuid = sqlx::query_scalar(
            "SELECT id
             FROM agent_config_revisions
             WHERE repository_id = $1 AND commit_sha = $2 AND config_hash = $3",
        )
        .bind(repository_id.as_uuid())
        .bind(commit.as_str())
        .bind(parsed.hash.as_str())
        .fetch_one(&mut **transaction)
        .await
        .map_err(storage)?;
        stored_id
    };
    Ok(AgentConfigRevisionId::from_uuid(stored_id))
}

pub async fn persist_repository_oci_image_revisions(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    project_id: ProjectId,
    source_commit: &CommitSha,
    images: &[InspectedRepositoryOciImage],
) -> Result<(), ForgeRepositoryError> {
    for image in images {
        // Keeping the catalog read and revision insert in the receive
        // transaction makes the selected base digest part of the immutable
        // source revision, rather than resolving a mutable key later.
        let base_reference: Option<String> = sqlx::query_scalar(
            "SELECT image_reference
             FROM oci_images
             WHERE key = $1 AND availability_state = 'available' AND role = 'execution'",
        )
        .bind(&image.base_key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?;
        let base_reference = base_reference.ok_or(ForgeRepositoryError::InvalidMetadata(
            "repository OCI image base is not an available catalog image",
        ))?;
        let revision_id = Uuid::new_v4();
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO repository_oci_image_definitions
                (id, project_id, source_repository_id, key, display_name,
                 source_revision, dockerfile_path, context_path, context_digest,
                 base_image_reference, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'producing')
             ON CONFLICT (source_repository_id, key, source_revision,
                          context_digest, base_image_reference)
             DO NOTHING
             RETURNING id",
        )
        .bind(revision_id)
        .bind(project_id.as_uuid())
        .bind(repository_id.as_uuid())
        .bind(&image.key)
        .bind(&image.display_name)
        .bind(source_commit.as_str())
        .bind(&image.dockerfile_path)
        .bind(&image.context_path)
        .bind(&image.context_digest)
        .bind(base_reference)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?;
    }
    Ok(())
}
