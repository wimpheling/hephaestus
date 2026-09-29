//! Loading and validating immutable UI captures.

use agent_config::build_identity::ui_build_definition_hash;
use agent_config::ui::RepositoryUisConfig;
use agent_config::ui::gateway_resolution::resolve_gateway_uis;
use agent_config::ui::static_resolution::resolve_static_uis;
use agent_config::{
    RepositoryGatewaysConfig, canonical_repository_gateways,
    validate_repository_uis_against_gateways,
};
use release_domain::BuildRequestId;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::{ReleaseServiceError, ResolvedUiPublication, UiCaptureRow, UiPublicationCandidates};

/// Loads and resolves the UI capture linked to one locked build request.
///
/// A build with no UI-source link returns `Ok(None)` for legacy behavior.  A
/// present link whose repository, commit, source row, hashes, or typed
/// references do not match returns one constant redacted stored-data error.
/// The build request itself is locked by the query; no UI rows are written.
pub async fn load_ui_publication(
    transaction: &mut Transaction<'_, Postgres>,
    build_request_id: BuildRequestId,
    candidates: UiPublicationCandidates<'_>,
) -> Result<Option<ResolvedUiPublication>, ReleaseServiceError> {
    let row = sqlx::query_as::<_, UiCaptureRow>(
        "SELECT request.repository_id AS request_repository_id,
                request.source_commit AS request_source_commit,
                request.build_definition_hash AS request_build_definition_hash,
                link.repository_id AS link_repository_id,
                link.source_commit AS link_source_commit,
                link.source_manifest_revision_id,
                capture.repository_id AS capture_repository_id,
                capture.source_commit AS capture_source_commit,
                capture.status AS capture_status,
                capture.normalized_ui_config,
                capture.normalized_ui_hash,
                capture.requires_gateways,
                capture.normalized_gateway_config,
                capture.normalized_gateway_hash
         FROM build_requests AS request
         LEFT JOIN build_request_ui_source_manifests AS link
           ON link.build_request_id = request.id
         LEFT JOIN ui_source_manifest_revisions AS capture
           ON capture.id = link.source_manifest_revision_id
          AND capture.repository_id = link.repository_id
          AND capture.source_commit = link.source_commit
         WHERE request.id = $1
         FOR UPDATE OF request",
    )
    .bind(build_request_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(ReleaseServiceError::Unavailable)?;

    if row.request_repository_id != candidates.repository_id.as_uuid() {
        return Err(invalid_ui_storage());
    }
    let Some(source_manifest_revision_id) = row.source_manifest_revision_id else {
        if row.link_repository_id.is_some() || row.link_source_commit.is_some() {
            return Err(invalid_ui_storage());
        }
        return Ok(None);
    };
    if row.link_repository_id != Some(row.request_repository_id)
        || row.link_source_commit.as_deref() != Some(row.request_source_commit.as_str())
        || row.capture_repository_id != Some(row.request_repository_id)
        || row.capture_source_commit.as_deref() != Some(row.request_source_commit.as_str())
        || row.capture_status.as_deref() != Some("valid")
    {
        return Err(invalid_ui_storage());
    }

    let base_build_definition_hash = candidates.base_build_definition_hash;
    let publication = resolve_captured_ui(
        source_manifest_revision_id,
        row.normalized_ui_config.ok_or_else(invalid_ui_storage)?,
        row.normalized_ui_hash.as_deref(),
        row.requires_gateways == Some(true),
        row.normalized_gateway_config,
        row.normalized_gateway_hash.as_deref(),
        candidates,
    )?;
    let base_build_definition_hash = base_build_definition_hash.ok_or_else(invalid_ui_storage)?;
    verify_build_definition_hash(
        &row.request_build_definition_hash,
        publication.normalized_ui_hash,
        publication.normalized_gateway_hash,
        base_build_definition_hash,
    )?;
    Ok(Some(publication))
}

pub(super) fn resolve_captured_ui(
    source_manifest_revision_id: Uuid,
    ui_json: Value,
    ui_hash: Option<&[u8]>,
    requires_gateways: bool,
    gateway_json: Option<Value>,
    gateway_hash: Option<&[u8]>,
    candidates: UiPublicationCandidates<'_>,
) -> Result<ResolvedUiPublication, ReleaseServiceError> {
    let config: RepositoryUisConfig =
        serde_json::from_value(ui_json).map_err(|_| invalid_ui_storage())?;
    let actual_ui_hash = decode_hash(ui_hash)?;
    if canonical_json_hash(&config)? != actual_ui_hash {
        return Err(invalid_ui_storage());
    }
    let (gateways, actual_gateway_hash) = if requires_gateways {
        let gateways: RepositoryGatewaysConfig =
            serde_json::from_value(gateway_json.ok_or_else(invalid_ui_storage)?)
                .map_err(|_| invalid_ui_storage())?;
        let hash = decode_hash(gateway_hash)?;
        if canonical_gateway_hash(&gateways)? != hash
            || !validate_repository_uis_against_gateways(&config, Some(&gateways)).is_empty()
        {
            return Err(invalid_ui_storage());
        }
        (Some(gateways), Some(hash))
    } else {
        if gateway_json.is_some() || gateway_hash.is_some() {
            return Err(invalid_ui_storage());
        }
        if !validate_repository_uis_against_gateways(&config, None).is_empty() {
            return Err(invalid_ui_storage());
        }
        (None, None)
    };
    let static_uis = resolve_static_uis(&config, candidates.static_artifacts)
        .map_err(|_| invalid_ui_storage())?;
    let gateway_uis = resolve_gateway_uis(&config, gateways.as_ref(), candidates.release_agents)
        .map_err(|_| invalid_ui_storage())?;
    Ok(ResolvedUiPublication {
        source_manifest_revision_id,
        config,
        normalized_ui_hash: actual_ui_hash,
        normalized_gateway_hash: actual_gateway_hash,
        static_uis,
        gateway_uis,
    })
}

const fn invalid_ui_storage() -> ReleaseServiceError {
    ReleaseServiceError::InvalidStoredData
}

fn decode_hash(value: Option<&[u8]>) -> Result<[u8; 32], ReleaseServiceError> {
    let value = value.ok_or_else(invalid_ui_storage)?;
    value.try_into().map_err(|_| invalid_ui_storage())
}

pub(super) fn canonical_json_hash(
    config: &RepositoryUisConfig,
) -> Result<[u8; 32], ReleaseServiceError> {
    let bytes = serde_json::to_vec(config).map_err(|_| invalid_ui_storage())?;
    Ok(Sha256::digest(bytes).into())
}

pub(super) fn canonical_gateway_hash(
    config: &RepositoryGatewaysConfig,
) -> Result<[u8; 32], ReleaseServiceError> {
    let canonical = canonical_repository_gateways(config);
    let bytes = toml::to_string(&canonical).map_err(|_| invalid_ui_storage())?;
    Ok(Sha256::digest(bytes.as_bytes()).into())
}

pub(super) fn verify_build_definition_hash(
    stored_build_definition_hash: &[u8],
    normalized_ui_hash: [u8; 32],
    normalized_gateway_hash: Option<[u8; 32]>,
    base_build_definition_hash: [u8; 32],
) -> Result<(), ReleaseServiceError> {
    let stored_build_definition_hash: [u8; 32] = stored_build_definition_hash
        .try_into()
        .map_err(|_| invalid_ui_storage())?;
    let expected = ui_build_definition_hash(
        base_build_definition_hash,
        normalized_ui_hash,
        normalized_gateway_hash,
    );
    if stored_build_definition_hash != expected {
        return Err(invalid_ui_storage());
    }
    Ok(())
}
