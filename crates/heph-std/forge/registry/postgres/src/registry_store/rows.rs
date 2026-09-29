use super::mapping::publication_from_rows;
use super::notification::RegistryStoreError;
use super::parsing::{owner_fields, storage};
use super::prelude::*;
#[derive(FromRow)]
pub(super) struct PublicationRow {
    pub(super) id: Uuid,
    pub(super) repository_path: String,
    pub(super) owner_kind: String,
    pub(super) platform_image_key: Option<String>,
    pub(super) owner_id: Option<Uuid>,
    pub(super) project_id: Option<Uuid>,
    pub(super) registry_authority: String,
    pub(super) expected_digest: String,
    pub(super) expected_media_type: String,
    pub(super) expected_size: i64,
    pub(super) policy_version: String,
    pub(super) signature_required: bool,
    pub(super) state: String,
    pub(super) verified_at: Option<OffsetDateTime>,
    pub(super) approved_at: Option<OffsetDateTime>,
}

#[derive(FromRow)]
pub(super) struct PlatformRow {
    pub(super) digest: String,
    pub(super) size: i64,
    pub(super) media_type: String,
    pub(super) operating_system: String,
    pub(super) architecture: String,
    pub(super) variant: Option<String>,
}

#[derive(FromRow)]
pub(super) struct EvidenceRow {
    pub(super) kind: String,
    pub(super) subject_digest: String,
    pub(super) digest: String,
    pub(super) size: i64,
    pub(super) media_type: String,
    pub(super) artifact_type: String,
}

#[derive(FromRow)]
pub(super) struct PublicationMetricsRow {
    pub(super) pending: i64,
    pub(super) publishing: i64,
    pub(super) verified: i64,
    pub(super) approved: i64,
    pub(super) retired: i64,
    pub(super) missing: i64,
}

#[derive(FromRow)]
pub(super) struct NotificationMetricsRow {
    pub(super) pending: i64,
    pub(super) claimed: i64,
    pub(super) expired_claims: i64,
    pub(super) processed: i64,
    pub(super) rejected: i64,
}

#[derive(FromRow)]
pub(super) struct NotificationRow {
    pub(super) id: Uuid,
    pub(super) event_key: String,
    pub(super) repository_path: String,
    pub(super) action: String,
    pub(super) target_digest: Option<String>,
    pub(super) target_media_type: Option<String>,
    pub(super) target_size: Option<i64>,
    pub(super) event_occurred_at: OffsetDateTime,
    pub(super) payload_sha256: Vec<u8>,
    pub(super) state: String,
    pub(super) claim_token: Option<Uuid>,
    pub(super) lease_expires_at: Option<OffsetDateTime>,
    pub(super) failure_code: Option<String>,
    pub(super) processed_at: Option<OffsetDateTime>,
}

pub(super) async fn ensure_namespace(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &NamespaceClaim,
) -> Result<Uuid, RegistryStoreError> {
    let (owner_kind, platform_image_key, owner_id, project_id) = owner_fields(claim.owner());
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO registry_namespaces (
            id, repository_path, owner_kind, platform_image_key, owner_id, project_id
         ) VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (repository_path) DO NOTHING RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(claim.namespace().as_str())
    .bind(owner_kind)
    .bind(platform_image_key)
    .bind(owner_id)
    .bind(project_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?;
    let id = if let Some(id) = inserted {
        id
    } else {
        let stored = sqlx::query_as::<_, NamespaceRow>(
            "SELECT id, repository_path, owner_kind, platform_image_key, owner_id, project_id
             FROM registry_namespaces WHERE repository_path = $1 FOR UPDATE",
        )
        .bind(claim.namespace().as_str())
        .fetch_one(&mut **transaction)
        .await
        .map_err(storage)?;
        if stored.repository_path != claim.namespace().as_str()
            || stored.owner_kind != owner_kind
            || stored.platform_image_key.as_deref() != platform_image_key
            || stored.owner_id != owner_id
            || stored.project_id != project_id
        {
            return Err(RegistryStoreError::Conflict);
        }
        stored.id
    };
    Ok(id)
}

#[derive(FromRow)]
pub(super) struct NamespaceRow {
    pub(super) id: Uuid,
    pub(super) repository_path: String,
    pub(super) owner_kind: String,
    pub(super) platform_image_key: Option<String>,
    pub(super) owner_id: Option<Uuid>,
    pub(super) project_id: Option<Uuid>,
}

pub(super) async fn load_intent(
    transaction: &mut Transaction<'_, Postgres>,
    id: Uuid,
    for_update: bool,
) -> Result<PublicationIntent, RegistryStoreError> {
    let row = if for_update {
        sqlx::query_as::<_, PublicationRow>(
            "SELECT publication.id, namespace.repository_path, namespace.owner_kind,
            namespace.platform_image_key, namespace.owner_id, namespace.project_id,
            publication.registry_authority, publication.expected_digest,
            publication.expected_media_type, publication.expected_size,
            publication.policy_version, publication.signature_required, publication.state
            , publication.verified_at, publication.approved_at
         FROM registry_publications publication
         JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id
         WHERE publication.id = $1 FOR UPDATE OF publication",
        )
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?
    } else {
        sqlx::query_as::<_, PublicationRow>(
            "SELECT publication.id, namespace.repository_path, namespace.owner_kind,
            namespace.platform_image_key, namespace.owner_id, namespace.project_id,
            publication.registry_authority, publication.expected_digest,
            publication.expected_media_type, publication.expected_size,
            publication.policy_version, publication.signature_required, publication.state
            , publication.verified_at, publication.approved_at
         FROM registry_publications publication
         JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id
         WHERE publication.id = $1",
        )
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?
    }
    .ok_or(RegistryStoreError::Conflict)?;
    let platforms = sqlx::query_as::<_, PlatformRow>(
        "SELECT digest, size, media_type, operating_system, architecture, variant
         FROM registry_publication_platforms WHERE publication_id = $1 ORDER BY digest",
    )
    .bind(id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(storage)?;
    let evidence = sqlx::query_as::<_, EvidenceRow>(
        "SELECT kind, subject_digest, digest, size, media_type, artifact_type
         FROM registry_publication_evidence WHERE publication_id = $1 ORDER BY kind, digest",
    )
    .bind(id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(storage)?;
    publication_from_rows(row, platforms, evidence)
}
