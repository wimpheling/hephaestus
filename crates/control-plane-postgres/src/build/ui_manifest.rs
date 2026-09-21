//! Transaction-local lookup and linking for captured UI source manifests.
//!
//! Consumes immutable capture rows produced by the forge adapter and resolves
//! manual request hashes against the trusted build declaration without reading Git.

use forge_domain::{CommitSha, RepositoryId};
use release_domain::BuildRequestId;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::BuildError;

const VALID_STATUS: &str = "valid";
const INVALID_STATUS: &str = "invalid";

/// The immutable UI capture state for one exact repository commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UiManifestSnapshot {
    /// No UI capture was recorded for this commit (legacy source).
    Absent,
    /// The source contained a UI declaration that failed validation.
    Invalid,
    /// A valid immutable capture and its normalized configuration hashes.
    Valid {
        /// The immutable capture revision to link to a build.
        revision_id: Uuid,
        /// SHA-256 of the normalized UI configuration.
        ui_hash: [u8; 32],
        /// SHA-256 of the normalized gateway configuration, when required.
        gateway_hash: Option<[u8; 32]>,
    },
}

#[derive(Debug, FromRow)]
struct UiManifestRevisionRow {
    id: Uuid,
    status: String,
    requires_gateways: bool,
    normalized_ui_hash: Option<Vec<u8>>,
    normalized_gateway_hash: Option<Vec<u8>>,
}

/// Looks up the immutable capture for an exact repository and commit.
pub(super) async fn lookup_ui_manifest_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    source_commit: &CommitSha,
) -> Result<UiManifestSnapshot, BuildError> {
    let row = sqlx::query_as::<_, UiManifestRevisionRow>(
        "SELECT id, status, requires_gateways, normalized_ui_hash,
                normalized_gateway_hash
         FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository_id.as_uuid())
    .bind(source_commit.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(BuildError::Persistence)?;

    row.map_or(Ok(UiManifestSnapshot::Absent), decode_snapshot)
}

/// Links a valid immutable capture to one exact build request.
pub(super) async fn link_ui_manifest_to_build(
    transaction: &mut Transaction<'_, Postgres>,
    build_request_id: BuildRequestId,
    repository_id: RepositoryId,
    source_commit: &CommitSha,
    snapshot: UiManifestSnapshot,
) -> Result<(), BuildError> {
    let UiManifestSnapshot::Valid { revision_id, .. } = snapshot else {
        return Err(BuildError::FailedPrecondition);
    };

    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id, source_status)
         VALUES ($1, $2, $3, $4, 'valid')
         ON CONFLICT (build_request_id) DO NOTHING",
    )
    .bind(build_request_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(source_commit.as_str())
    .bind(revision_id)
    .execute(&mut **transaction)
    .await
    .map_err(BuildError::Persistence)?;

    let existing: Option<BuildUiManifestLinkRow> = sqlx::query_as(
        "SELECT build_request_id, repository_id, source_commit,
                source_manifest_revision_id, source_status
         FROM build_request_ui_source_manifests
         WHERE build_request_id = $1",
    )
    .bind(build_request_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(BuildError::Persistence)?;
    let Some(existing) = existing else {
        return Err(BuildError::InvalidStoredData);
    };
    if existing.build_request_id != build_request_id.as_uuid()
        || existing.repository_id != repository_id.as_uuid()
        || existing.source_commit != source_commit.as_str()
        || existing.source_manifest_revision_id != revision_id
        || existing.source_status != VALID_STATUS
    {
        return Err(BuildError::InvalidStoredData);
    }
    Ok(())
}

/// Recomputes and accepts the trusted base or UI-aware build identity.
pub(super) fn resolve_build_definition_hash(
    snapshot: UiManifestSnapshot,
    base_hash: [u8; 32],
    requested_hash: [u8; 32],
) -> Result<[u8; 32], BuildError> {
    match snapshot {
        UiManifestSnapshot::Absent => (requested_hash == base_hash)
            .then_some(base_hash)
            .ok_or(BuildError::FailedPrecondition),
        UiManifestSnapshot::Invalid => Err(BuildError::FailedPrecondition),
        UiManifestSnapshot::Valid {
            ui_hash,
            gateway_hash,
            ..
        } => {
            let derived_hash = agent_config::build_identity::ui_build_definition_hash(
                base_hash,
                ui_hash,
                gateway_hash,
            );
            (requested_hash == base_hash || requested_hash == derived_hash)
                .then_some(derived_hash)
                .ok_or(BuildError::FailedPrecondition)
        }
    }
}

#[derive(Debug, FromRow)]
struct BuildUiManifestLinkRow {
    build_request_id: Uuid,
    repository_id: Uuid,
    source_commit: String,
    source_manifest_revision_id: Uuid,
    source_status: String,
}

fn decode_snapshot(row: UiManifestRevisionRow) -> Result<UiManifestSnapshot, BuildError> {
    match row.status.as_str() {
        INVALID_STATUS => Ok(UiManifestSnapshot::Invalid),
        VALID_STATUS => decode_valid_snapshot(row),
        _ => Err(BuildError::InvalidStoredData),
    }
}

fn decode_valid_snapshot(row: UiManifestRevisionRow) -> Result<UiManifestSnapshot, BuildError> {
    let ui_hash = decode_hash(row.normalized_ui_hash)?.ok_or(BuildError::InvalidStoredData)?;

    let gateway_hash = if row.requires_gateways {
        Some(decode_hash(row.normalized_gateway_hash)?.ok_or(BuildError::InvalidStoredData)?)
    } else {
        if row.normalized_gateway_hash.is_some() {
            return Err(BuildError::InvalidStoredData);
        }
        None
    };

    Ok(UiManifestSnapshot::Valid {
        revision_id: row.id,
        ui_hash,
        gateway_hash,
    })
}

fn decode_hash(value: Option<Vec<u8>>) -> Result<Option<[u8; 32]>, BuildError> {
    value
        .map(|bytes| bytes.try_into().map_err(|_| BuildError::InvalidStoredData))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> UiManifestRevisionRow {
        UiManifestRevisionRow {
            id: Uuid::new_v4(),
            status: VALID_STATUS.to_owned(),
            requires_gateways: false,
            normalized_ui_hash: Some(vec![2; 32]),
            normalized_gateway_hash: None,
        }
    }

    #[test]
    fn decodes_valid_ui_and_hashes() {
        let valid_row = row();
        let revision_id = valid_row.id;
        assert_eq!(
            decode_snapshot(valid_row).expect("valid row"),
            UiManifestSnapshot::Valid {
                revision_id,
                ui_hash: [2; 32],
                gateway_hash: None,
            }
        );
    }

    #[test]
    fn preserves_invalid_as_typed_state() {
        let mut invalid = row();
        invalid.status = INVALID_STATUS.to_owned();
        invalid.normalized_ui_hash = None;
        assert_eq!(
            decode_snapshot(invalid).expect("invalid is a source result"),
            UiManifestSnapshot::Invalid
        );
    }

    #[test]
    fn rejects_hash_and_gateway_shape_mismatches() {
        let mut bad_hash = row();
        bad_hash.normalized_ui_hash = Some(vec![2; 31]);
        assert!(matches!(
            decode_snapshot(bad_hash),
            Err(BuildError::InvalidStoredData)
        ));

        let mut bad_gateways = row();
        bad_gateways.requires_gateways = true;
        assert!(matches!(
            decode_snapshot(bad_gateways),
            Err(BuildError::InvalidStoredData)
        ));
    }

    #[test]
    fn accepts_base_or_derived_identity_but_stores_derived() {
        let snapshot = UiManifestSnapshot::Valid {
            revision_id: Uuid::new_v4(),
            ui_hash: [2; 32],
            gateway_hash: Some([3; 32]),
        };
        let base = [1; 32];
        let derived =
            agent_config::build_identity::ui_build_definition_hash(base, [2; 32], Some([3; 32]));
        assert_eq!(
            resolve_build_definition_hash(snapshot, base, base).expect("base accepted"),
            derived
        );
        assert_eq!(
            resolve_build_definition_hash(snapshot, base, derived).expect("derived accepted"),
            derived
        );
        assert!(matches!(
            resolve_build_definition_hash(snapshot, base, [9; 32]),
            Err(BuildError::FailedPrecondition)
        ));
    }

    #[test]
    fn invalid_snapshot_never_accepts_a_build_identity() {
        assert!(matches!(
            resolve_build_definition_hash(UiManifestSnapshot::Invalid, [1; 32], [1; 32]),
            Err(BuildError::FailedPrecondition)
        ));
    }
}
