//! Transactional persistence for one inspected repository UI manifest.
//!
//! It stores bounded source evidence and reuses an identical immutable revision
//! when the same repository commit is observed by another receive.

use agent_config::ConfigHash;
use forge_domain::{CommitSha, ReceiveId, RepositoryId};
use forge_service::ForgeRepositoryError;
use release_domain::BuildRequestId;
use serde_json::Value;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::ui_manifest::{UiManifestEntryKind, UiManifestInspection, UiManifestStatus};

#[cfg(test)]
mod tests;

/// The immutable revision identity and status needed by later build linking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StoredUiManifestRevision {
    /// Durable revision identity.
    pub(super) id: Uuid,
    /// Whether the captured source passed UI validation.
    pub(super) status: UiManifestStatus,
    /// SHA-256 of the normalized UI configuration, when valid.
    pub(super) normalized_ui_hash: Option<[u8; 32]>,
    /// SHA-256 of the normalized gateway configuration, when valid.
    pub(super) normalized_gateway_hash: Option<[u8; 32]>,
}

#[derive(Debug, FromRow)]
struct ExistingUiManifestRevision {
    id: Uuid,
    entry_kind: String,
    manifest_oid: String,
    actual_size_bytes: Option<i64>,
    source_sha256: Option<Vec<u8>>,
    status: String,
    requires_gateways: bool,
    normalized_ui_config: Option<Value>,
    normalized_ui_hash: Option<Vec<u8>>,
    gateway_manifest_oid: Option<String>,
    gateway_actual_size_bytes: Option<i64>,
    gateway_source_sha256: Option<Vec<u8>>,
    normalized_gateway_config: Option<Value>,
    normalized_gateway_hash: Option<Vec<u8>>,
    diagnostics: Value,
}

#[derive(Debug)]
struct UiManifestEvidence {
    entry_kind: &'static str,
    manifest_oid: String,
    actual_size_bytes: Option<i64>,
    source_sha256: Option<Vec<u8>>,
    status: &'static str,
    requires_gateways: bool,
    normalized_ui_config: Option<Value>,
    normalized_ui_hash: Option<Vec<u8>>,
    gateway_manifest_oid: Option<String>,
    gateway_actual_size_bytes: Option<i64>,
    gateway_source_sha256: Option<Vec<u8>>,
    normalized_gateway_config: Option<Value>,
    normalized_gateway_hash: Option<Vec<u8>>,
    diagnostics: Value,
}

/// Persists one inspected UI manifest in the caller's receive transaction.
///
/// An existing `(repository_id, source_commit)` row is never updated.  Equal
/// evidence reuses it, while a mismatch fails closed so parser or Git evidence
/// cannot silently replace the original receive provenance.
pub(super) async fn persist_ui_manifest_revision(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    receive_id: ReceiveId,
    source_commit: &CommitSha,
    inspection: &UiManifestInspection,
) -> Result<StoredUiManifestRevision, ForgeRepositoryError> {
    let evidence = evidence(inspection)?;
    let id = Uuid::new_v4();
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, manifest_path,
          entry_kind, manifest_oid, actual_size_bytes, source_sha256, status,
          requires_gateways, normalized_ui_config, normalized_ui_hash,
          gateway_manifest_oid, gateway_actual_size_bytes,
          gateway_source_sha256, normalized_gateway_config,
          normalized_gateway_hash, diagnostics)
         VALUES ($1, $2, $3, $4, 'heph.ui.toml', $5, $6, $7, $8, $9,
                 $10, $11, $12, $13, $14, $15, $16, $17, $18)
         ON CONFLICT (repository_id, source_commit) DO NOTHING
         RETURNING id",
    )
    .bind(id)
    .bind(repository_id.as_uuid())
    .bind(receive_id.as_uuid())
    .bind(source_commit.as_str())
    .bind(evidence.entry_kind)
    .bind(&evidence.manifest_oid)
    .bind(evidence.actual_size_bytes)
    .bind(evidence.source_sha256.clone())
    .bind(evidence.status)
    .bind(evidence.requires_gateways)
    .bind(evidence.normalized_ui_config.clone())
    .bind(evidence.normalized_ui_hash.clone())
    .bind(evidence.gateway_manifest_oid.clone())
    .bind(evidence.gateway_actual_size_bytes)
    .bind(evidence.gateway_source_sha256.clone())
    .bind(evidence.normalized_gateway_config.clone())
    .bind(evidence.normalized_gateway_hash.clone())
    .bind(evidence.diagnostics.clone())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(super::storage)?;

    if let Some(id) = inserted {
        return Ok(StoredUiManifestRevision {
            id,
            status: inspection.status,
            normalized_ui_hash: hash_array(evidence.normalized_ui_hash, "UI manifest hash")?,
            normalized_gateway_hash: hash_array(
                evidence.normalized_gateway_hash,
                "gateway manifest hash",
            )?,
        });
    }

    let existing = sqlx::query_as::<_, ExistingUiManifestRevision>(
        "SELECT id, entry_kind, manifest_oid, actual_size_bytes,
                source_sha256, status, requires_gateways,
                normalized_ui_config, normalized_ui_hash,
                gateway_manifest_oid, gateway_actual_size_bytes,
                gateway_source_sha256, normalized_gateway_config,
                normalized_gateway_hash, diagnostics
         FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository_id.as_uuid())
    .bind(source_commit.as_str())
    .fetch_one(&mut **transaction)
    .await
    .map_err(super::storage)?;

    if !same_evidence(&existing, &evidence) {
        return Err(ForgeRepositoryError::InvalidMetadata(
            "conflicting repository UI capture evidence",
        ));
    }

    let status = parse_status(&existing.status)?;
    Ok(StoredUiManifestRevision {
        id: existing.id,
        status,
        normalized_ui_hash: hash_array(existing.normalized_ui_hash, "UI manifest hash")?,
        normalized_gateway_hash: hash_array(
            existing.normalized_gateway_hash,
            "gateway manifest hash",
        )?,
    })
}

/// Links a valid immutable UI revision to its exact build request.
pub(super) async fn link_ui_manifest_to_build(
    transaction: &mut Transaction<'_, Postgres>,
    build_request_id: BuildRequestId,
    repository_id: RepositoryId,
    source_commit: &CommitSha,
    revision_id: Uuid,
) -> Result<(), ForgeRepositoryError> {
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (build_request_id) DO NOTHING",
    )
    .bind(build_request_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(source_commit.as_str())
    .bind(revision_id)
    .execute(&mut **transaction)
    .await
    .map_err(super::storage)?;

    let existing: (Uuid, Uuid, String, Uuid) = sqlx::query_as(
        "SELECT build_request_id, repository_id, source_commit,
                source_manifest_revision_id
         FROM build_request_ui_source_manifests
         WHERE build_request_id = $1",
    )
    .bind(build_request_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(super::storage)?;
    if existing.1 != repository_id.as_uuid()
        || existing.2 != source_commit.as_str()
        || existing.3 != revision_id
    {
        return Err(ForgeRepositoryError::InvalidMetadata(
            "conflicting repository UI build link",
        ));
    }
    Ok(())
}

fn evidence(inspection: &UiManifestInspection) -> Result<UiManifestEvidence, ForgeRepositoryError> {
    let actual_size_bytes = inspection
        .actual_size
        .map(i64::try_from)
        .transpose()
        .map_err(|_| ForgeRepositoryError::InvalidStoredData("UI manifest size"))?;
    let gateway_actual_size_bytes = inspection
        .gateway_actual_size
        .map(i64::try_from)
        .transpose()
        .map_err(|_| ForgeRepositoryError::InvalidStoredData("gateway manifest size"))?;
    let normalized_ui_config = inspection
        .config
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(super::serialization)?;
    let normalized_gateway_config = inspection
        .gateway_config
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(super::serialization)?;

    Ok(UiManifestEvidence {
        entry_kind: entry_kind(inspection.entry_kind),
        manifest_oid: inspection.object_id.to_hex().to_string(),
        actual_size_bytes,
        source_sha256: hash_bytes(inspection.source_hash.as_ref())?,
        status: status(inspection.status),
        requires_gateways: inspection.requires_gateways,
        normalized_ui_config,
        normalized_ui_hash: hash_bytes(inspection.normalized_hash.as_ref())?,
        gateway_manifest_oid: inspection
            .gateway_object_id
            .map(|value| value.to_hex().to_string()),
        gateway_actual_size_bytes,
        gateway_source_sha256: hash_bytes(inspection.gateway_source_hash.as_ref())?,
        normalized_gateway_config,
        normalized_gateway_hash: hash_bytes(inspection.gateway_normalized_hash.as_ref())?,
        diagnostics: serde_json::to_value(&inspection.diagnostics).map_err(super::serialization)?,
    })
}

fn same_evidence(existing: &ExistingUiManifestRevision, expected: &UiManifestEvidence) -> bool {
    existing.entry_kind == expected.entry_kind
        && existing.manifest_oid == expected.manifest_oid
        && existing.actual_size_bytes == expected.actual_size_bytes
        && existing.source_sha256 == expected.source_sha256
        && existing.status == expected.status
        && existing.requires_gateways == expected.requires_gateways
        && existing.normalized_ui_config == expected.normalized_ui_config
        && existing.normalized_ui_hash == expected.normalized_ui_hash
        && existing.gateway_manifest_oid == expected.gateway_manifest_oid
        && existing.gateway_actual_size_bytes == expected.gateway_actual_size_bytes
        && existing.gateway_source_sha256 == expected.gateway_source_sha256
        && existing.normalized_gateway_config == expected.normalized_gateway_config
        && existing.normalized_gateway_hash == expected.normalized_gateway_hash
        && existing.diagnostics == expected.diagnostics
}

fn hash_bytes(hash: Option<&ConfigHash>) -> Result<Option<Vec<u8>>, ForgeRepositoryError> {
    hash.map(|value| {
        let text = value.as_str();
        if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ForgeRepositoryError::InvalidStoredData("UI manifest hash"));
        }
        (0..32)
            .map(|index| {
                u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
                    .map_err(|_| ForgeRepositoryError::InvalidStoredData("UI manifest hash"))
            })
            .collect()
    })
    .transpose()
}

fn hash_array(
    value: Option<Vec<u8>>,
    field: &'static str,
) -> Result<Option<[u8; 32]>, ForgeRepositoryError> {
    value
        .map(|bytes| {
            bytes
                .try_into()
                .map_err(|_| ForgeRepositoryError::InvalidStoredData(field))
        })
        .transpose()
}

const fn entry_kind(kind: UiManifestEntryKind) -> &'static str {
    match kind {
        UiManifestEntryKind::Regular => "blob",
        UiManifestEntryKind::Symlink => "symlink",
        UiManifestEntryKind::Tree => "tree",
        UiManifestEntryKind::Gitlink => "gitlink",
    }
}

const fn status(status: UiManifestStatus) -> &'static str {
    match status {
        UiManifestStatus::Valid => "valid",
        UiManifestStatus::Invalid => "invalid",
    }
}

fn parse_status(value: &str) -> Result<UiManifestStatus, ForgeRepositoryError> {
    match value {
        "valid" => Ok(UiManifestStatus::Valid),
        "invalid" => Ok(UiManifestStatus::Invalid),
        _ => Err(ForgeRepositoryError::InvalidStoredData(
            "UI manifest status",
        )),
    }
}
