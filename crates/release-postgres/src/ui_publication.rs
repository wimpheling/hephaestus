//! Release-publication preparation for repository UIs.
//!
//! This module loads the immutable UI capture attached to one exact build
//! request, validates its stored canonical hashes, resolves references to
//! caller-supplied release artifact and agent identities, and persists the
//! resulting release-owned UI bindings in the caller's publication transaction.

use agent_config::build_identity::ui_build_definition_hash;
use agent_config::ui::RepositoryUisConfig;
use agent_config::ui::gateway_resolution::{
    ReleaseAgentBinding, ResolvedGatewayUis, resolve_gateway_uis,
};
use agent_config::ui::static_resolution::{
    ResolvedStaticUis, StaticArtifactCandidate, resolve_static_uis,
};
use agent_config::ui::{UiCachePolicy, UiContent};
use agent_config::{
    RepositoryGatewaysConfig, canonical_repository_gateways,
    validate_repository_uis_against_gateways, validate_ui_route_collisions,
};
use forge_domain::RepositoryId;
use release_domain::ui::{UiIcon, UiPresentation, UiScope};
use release_domain::{BuildRequestId, ReleaseId};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::ReleaseServiceError;

/// Inputs supplied by the trusted build importer for one exact release.
#[derive(Clone, Copy)]
pub struct UiPublicationCandidates<'a> {
    /// Exact repository owning the locked build request.
    pub repository_id: RepositoryId,
    /// Base build-definition hash derived from the loaded agent configuration.
    ///
    /// Legacy configurations may omit a build declaration. This remains
    /// absent for a legacy build with no UI link, but is required once a
    /// linked UI capture reaches identity verification.
    pub base_build_definition_hash: Option<[u8; 32]>,
    /// Static artifact rows imported for the release candidate.
    pub static_artifacts: &'a [StaticArtifactCandidate],
    /// Exact exported release-agent key/ID bindings for this release.
    pub release_agents: &'a [ReleaseAgentBinding],
}

/// Fully validated and identity-resolved UI publication input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedUiPublication {
    /// Immutable source-capture row to link from the release.
    pub source_manifest_revision_id: Uuid,
    /// Typed, normalized UI configuration.
    pub config: RepositoryUisConfig,
    /// Hash retained by the source-capture row.
    pub normalized_ui_hash: [u8; 32],
    /// Canonical gateway hash when the UI references gateways.
    pub normalized_gateway_hash: Option<[u8; 32]>,
    /// Static routes resolved to exact release artifact IDs.
    pub static_uis: ResolvedStaticUis,
    /// Managed/API routes resolved to exact release-agent IDs.
    pub gateway_uis: ResolvedGatewayUis,
}

#[derive(Debug, sqlx::FromRow)]
struct UiCaptureRow {
    request_repository_id: Uuid,
    request_source_commit: String,
    request_build_definition_hash: Vec<u8>,
    link_repository_id: Option<Uuid>,
    link_source_commit: Option<String>,
    source_manifest_revision_id: Option<Uuid>,
    capture_repository_id: Option<Uuid>,
    capture_source_commit: Option<String>,
    capture_status: Option<String>,
    normalized_ui_config: Option<Value>,
    normalized_ui_hash: Option<Vec<u8>>,
    requires_gateways: Option<bool>,
    normalized_gateway_config: Option<Value>,
    normalized_gateway_hash: Option<Vec<u8>>,
}

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

fn resolve_captured_ui(
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

fn canonical_json_hash(config: &RepositoryUisConfig) -> Result<[u8; 32], ReleaseServiceError> {
    let bytes = serde_json::to_vec(config).map_err(|_| invalid_ui_storage())?;
    Ok(Sha256::digest(bytes).into())
}

fn canonical_gateway_hash(
    config: &RepositoryGatewaysConfig,
) -> Result<[u8; 32], ReleaseServiceError> {
    let canonical = canonical_repository_gateways(config);
    let bytes = toml::to_string(&canonical).map_err(|_| invalid_ui_storage())?;
    Ok(Sha256::digest(bytes.as_bytes()).into())
}

fn verify_build_definition_hash(
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

/// Persists one fully resolved UI publication into the caller's draft-release
/// transaction. Every child row uses the exact release, build, artifact, and
/// release-agent identities supplied by the trusted resolver; this helper
/// performs no idempotent updates and allocates no publication identities.
/// The ordered inserts preserve the publication foreign-key and draft-state
/// invariants in one caller-owned transaction.
// Keep the five ordered SQL inserts together so the publication invariant is
// visible at the transaction boundary.
#[allow(clippy::too_many_lines)]
pub async fn persist_ui_publication(
    transaction: &mut Transaction<'_, Postgres>,
    release_id: ReleaseId,
    build_request_id: BuildRequestId,
    publication: &ResolvedUiPublication,
) -> Result<(), ReleaseServiceError> {
    if !validate_ui_route_collisions(&publication.config).is_empty() {
        return Err(invalid_ui_storage());
    }
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(release_id.as_uuid())
    .bind(build_request_id.as_uuid())
    .bind(publication.source_manifest_revision_id)
    .execute(&mut **transaction)
    .await?;

    for ui in &publication.config.uis {
        let resolved_gateway = publication
            .gateway_uis
            .uis
            .iter()
            .find(|candidate| candidate.key == ui.key)
            .ok_or_else(invalid_ui_storage)?;
        let (content_kind, entrypoint) = match &ui.content {
            UiContent::Static { entrypoint, .. } => ("static", entrypoint.as_str()),
            UiContent::ManagedService { entrypoint, .. } => {
                ("managed_service", entrypoint.as_str())
            }
        };
        sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind,
              repository_git_access)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(release_id.as_uuid())
        .bind(ui.key.as_str())
        .bind(ui_scope_name(ui.scope))
        .bind(ui.label.as_str())
        .bind(ui_icon_name(ui.icon))
        .bind(ui_presentation_name(ui.presentation))
        .bind(ui.route_base.as_str())
        .bind(entrypoint)
        .bind(i32::from(ui.ui_kit_version))
        .bind(ui_cache_name(ui.cache))
        .bind(content_kind)
        .bind(ui.repository_git_access.as_str())
        .execute(&mut **transaction)
        .await?;

        match &ui.content {
            UiContent::Static { .. } => {
                let static_ui = publication
                    .static_uis
                    .uis
                    .iter()
                    .find(|candidate| candidate.key == ui.key)
                    .ok_or_else(invalid_ui_storage)?;
                for file in &static_ui.files {
                    sqlx::query(
                        "INSERT INTO release_ui_static_files
                         (release_id, ui_key, content_kind, route, artifact_id,
                          artifact_kind, artifact_media_type)
                         VALUES ($1, $2, 'static', $3, $4, 'file', $5)",
                    )
                    .bind(release_id.as_uuid())
                    .bind(ui.key.as_str())
                    .bind(file.route.as_str())
                    .bind(file.artifact_id.as_uuid())
                    .bind(file.media_type.as_str())
                    .execute(&mut **transaction)
                    .await?;
                }
            }
            UiContent::ManagedService { .. } => {
                let managed = resolved_gateway
                    .managed_service
                    .as_ref()
                    .ok_or_else(invalid_ui_storage)?;
                sqlx::query(
                    "INSERT INTO release_ui_managed_services
                     (release_id, ui_key, content_kind, gateway_name, route,
                      release_agent_id)
                     VALUES ($1, $2, 'managed_service', $3, $4, $5)",
                )
                .bind(release_id.as_uuid())
                .bind(ui.key.as_str())
                .bind(managed.gateway_name.as_str())
                .bind(managed.route.as_str())
                .bind(managed.release_agent_id.as_uuid())
                .execute(&mut **transaction)
                .await?;
            }
        }

        for api in &resolved_gateway.apis {
            sqlx::query(
                "INSERT INTO release_ui_api_bindings
                 (release_id, ui_key, api_key, gateway_name, method, route,
                  release_agent_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(release_id.as_uuid())
            .bind(ui.key.as_str())
            .bind(api.key.as_str())
            .bind(api.gateway_name.as_str())
            .bind(http_method_name(api.method))
            .bind(api.route.as_str())
            .bind(api.release_agent_id.as_uuid())
            .execute(&mut **transaction)
            .await?;
        }
    }
    Ok(())
}

const fn ui_scope_name(scope: UiScope) -> &'static str {
    match scope {
        UiScope::Project => "project",
        UiScope::Repository => "repository",
        UiScope::Global => "global",
    }
}

const fn ui_presentation_name(presentation: UiPresentation) -> &'static str {
    match presentation {
        UiPresentation::Iframe => "iframe",
        UiPresentation::FullPage => "full_page",
    }
}

const fn ui_icon_name(icon: UiIcon) -> &'static str {
    icon.as_str()
}

const fn ui_cache_name(cache: UiCachePolicy) -> &'static str {
    match cache {
        UiCachePolicy::NoStore => "no_store",
    }
}

const fn http_method_name(method: gateway_domain::HttpMethod) -> &'static str {
    match method {
        gateway_domain::HttpMethod::Get => "GET",
        gateway_domain::HttpMethod::Post => "POST",
        gateway_domain::HttpMethod::Put => "PUT",
        gateway_domain::HttpMethod::Patch => "PATCH",
        gateway_domain::HttpMethod::Delete => "DELETE",
        gateway_domain::HttpMethod::Head => "HEAD",
        gateway_domain::HttpMethod::Options => "OPTIONS",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ResolvedUiPublication, UiPublicationCandidates, canonical_gateway_hash,
        canonical_json_hash, persist_ui_publication, resolve_captured_ui,
        verify_build_definition_hash,
    };
    use agent_config::ui::gateway_resolution::ReleaseAgentBinding;
    use agent_config::ui::gateway_resolution::ResolvedGatewayUis;
    use agent_config::ui::static_resolution::ResolvedStaticUis;
    use agent_config::ui::static_resolution::StaticArtifactCandidate;
    use agent_config::{parse_repository_gateways, parse_repository_uis};
    use gateway_domain::RoutePath;
    use release_domain::ui::UiRoutePath;
    use release_domain::{
        AgentKey, ArtifactKind, ArtifactPath, BuildRequestId, ReleaseAgentId, ReleaseArtifactId,
        ReleaseId,
    };
    use serial_test::serial;
    use sqlx::{Postgres, Transaction, postgres::PgPoolOptions};
    use std::{env, time::Duration};
    use uuid::Uuid;

    const UI: &str = r#"
version = 1

[[uis]]
key = "docs"
scope = "global"
label = "Docs"
icon = "book"
presentation = "iframe"
route_base = "docs"
ui_kit_version = 1
cache = "no_store"

[uis.content]
kind = "static"
entrypoint = "index.html"

[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"

[[uis]]
key = "assistant"
scope = "project"
label = "Assistant"
icon = "chat"
presentation = "iframe"
route_base = "assistant"
ui_kit_version = 1
cache = "no_store"

[[uis.apis]]
key = "service-api"
gateway_name = "ui-service"
method = "GET"
route = "/service/api"

[uis.content]
kind = "managed_service"
gateway_name = "ui-service"
route = "/service/ui"
entrypoint = "index.html"
"#;

    const GATEWAYS: &str = r#"
version = 1

[[gateways]]
name = "ui-service"
agent_name = "ui-service-agent"
handler_contract = "http.service.v1"
exposure = "heph_authenticated"

[gateways.service]
loopback_port = 8080
readiness_path = "/ready"
health_path = "/health"

[[gateways.routes]]
path = "/service"
methods = ["GET"]
"#;

    fn candidates() -> (Vec<StaticArtifactCandidate>, Vec<ReleaseAgentBinding>) {
        (
            vec![StaticArtifactCandidate {
                path: ArtifactPath::parse("dist/index.html").expect("artifact path"),
                id: ReleaseArtifactId::from_uuid(Uuid::from_u128(1)),
                kind: ArtifactKind::File,
                media_type: String::from("text/html"),
                size_bytes: 1,
            }],
            vec![ReleaseAgentBinding {
                agent_key: AgentKey::parse("ui-service-agent").expect("agent key"),
                release_agent_id: ReleaseAgentId::from_uuid(Uuid::from_u128(2)),
            }],
        )
    }

    fn captured() -> (serde_json::Value, Vec<u8>, serde_json::Value, Vec<u8>) {
        let ui = parse_repository_uis(UI.as_bytes())
            .config
            .expect("valid UI config");
        let gateways = parse_repository_gateways(GATEWAYS.as_bytes())
            .config
            .expect("valid gateway config");
        let ui_hash = canonical_json_hash(&ui).expect("UI hash");
        let gateway_hash = canonical_gateway_hash(&gateways).expect("gateway hash");
        (
            serde_json::to_value(ui).expect("UI JSON"),
            ui_hash.to_vec(),
            serde_json::to_value(gateways).expect("gateway JSON"),
            gateway_hash.to_vec(),
        )
    }

    #[test]
    fn valid_capture_resolves_exact_artifact_and_agent_ids() {
        let (ui_json, ui_hash, gateway_json, gateway_hash) = captured();
        let (artifacts, agents) = candidates();
        let result = resolve_captured_ui(
            Uuid::from_u128(3),
            ui_json,
            Some(ui_hash.as_slice()),
            true,
            Some(gateway_json),
            Some(gateway_hash.as_slice()),
            UiPublicationCandidates {
                repository_id: forge_domain::RepositoryId::from_uuid(Uuid::from_u128(4)),
                base_build_definition_hash: Some([11; 32]),
                static_artifacts: &artifacts,
                release_agents: &agents,
            },
        )
        .expect("valid captured UI");
        assert_eq!(result.source_manifest_revision_id, Uuid::from_u128(3));
        assert_eq!(
            result
                .static_uis
                .uis
                .iter()
                .find(|ui| ui.key.as_str() == "docs")
                .expect("static UI")
                .files[0]
                .artifact_id
                .as_uuid(),
            Uuid::from_u128(1)
        );
        assert_eq!(
            result
                .gateway_uis
                .uis
                .iter()
                .find(|ui| ui.key.as_str() == "assistant")
                .expect("managed UI")
                .managed_service
                .as_ref()
                .expect("managed binding")
                .release_agent_id,
            ReleaseAgentId::from_uuid(Uuid::from_u128(2))
        );
    }

    #[test]
    fn hash_mismatch_and_missing_artifact_or_agent_are_redacted_failures() {
        let (ui_json, mut ui_hash, gateway_json, gateway_hash) = captured();
        let (artifacts, agents) = candidates();
        ui_hash[0] ^= 1;
        let error = resolve_captured_ui(
            Uuid::from_u128(5),
            ui_json.clone(),
            Some(ui_hash.as_slice()),
            true,
            Some(gateway_json.clone()),
            Some(gateway_hash.as_slice()),
            UiPublicationCandidates {
                repository_id: forge_domain::RepositoryId::from_uuid(Uuid::from_u128(6)),
                base_build_definition_hash: Some([11; 32]),
                static_artifacts: &artifacts,
                release_agents: &agents,
            },
        )
        .expect_err("hash mismatch");
        assert!(matches!(
            error,
            super::ReleaseServiceError::InvalidStoredData
        ));

        let error = resolve_captured_ui(
            Uuid::from_u128(7),
            ui_json.clone(),
            Some(
                canonical_json_hash(&serde_json::from_value(ui_json.clone()).expect("typed UI"))
                    .expect("UI hash")
                    .as_slice(),
            ),
            true,
            Some(gateway_json.clone()),
            Some(gateway_hash.as_slice()),
            UiPublicationCandidates {
                repository_id: forge_domain::RepositoryId::from_uuid(Uuid::from_u128(8)),
                base_build_definition_hash: Some([11; 32]),
                static_artifacts: &[],
                release_agents: &agents,
            },
        )
        .expect_err("missing artifact");
        assert!(matches!(
            error,
            super::ReleaseServiceError::InvalidStoredData
        ));

        let error = resolve_captured_ui(
            Uuid::from_u128(9),
            ui_json.clone(),
            Some(
                canonical_json_hash(&serde_json::from_value(ui_json).expect("typed UI"))
                    .expect("UI hash")
                    .as_slice(),
            ),
            true,
            Some(gateway_json),
            Some(gateway_hash.as_slice()),
            UiPublicationCandidates {
                repository_id: forge_domain::RepositoryId::from_uuid(Uuid::from_u128(10)),
                base_build_definition_hash: Some([11; 32]),
                static_artifacts: &artifacts,
                release_agents: &[],
            },
        )
        .expect_err("missing agent");
        assert!(matches!(
            error,
            super::ReleaseServiceError::InvalidStoredData
        ));
    }

    #[test]
    fn derived_build_identity_must_match_the_stored_request_hash() {
        let base = [11; 32];
        let ui = [12; 32];
        let gateway = Some([13; 32]);
        let expected = agent_config::build_identity::ui_build_definition_hash(base, ui, gateway);
        verify_build_definition_hash(&expected, ui, gateway, base)
            .expect("matching derived build identity");
        let mut wrong = expected;
        wrong[0] ^= 1;
        assert!(matches!(
            verify_build_definition_hash(&wrong, ui, gateway, base),
            Err(super::ReleaseServiceError::InvalidStoredData)
        ));
    }

    fn publication_with_config(
        config: agent_config::ui::RepositoryUisConfig,
    ) -> ResolvedUiPublication {
        ResolvedUiPublication {
            source_manifest_revision_id: Uuid::new_v4(),
            config,
            normalized_ui_hash: [0; 32],
            normalized_gateway_hash: None,
            static_uis: ResolvedStaticUis { uis: Vec::new() },
            gateway_uis: ResolvedGatewayUis { uis: Vec::new() },
        }
    }

    async fn assert_no_publication_rows(
        transaction: &mut Transaction<'_, Postgres>,
        release_id: ReleaseId,
    ) {
        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM (
                   SELECT 1 FROM release_ui_source_snapshots WHERE release_id = $1
                   UNION ALL
                   SELECT 1 FROM release_ui_descriptors WHERE release_id = $1
                   UNION ALL
                   SELECT 1 FROM release_ui_static_files WHERE release_id = $1
                   UNION ALL
                   SELECT 1 FROM release_ui_managed_services WHERE release_id = $1
                   UNION ALL
                   SELECT 1 FROM release_ui_api_bindings WHERE release_id = $1
               ) AS publication_rows",
        )
        .bind(release_id.as_uuid())
        .fetch_one(&mut **transaction)
        .await
        .expect("count publication rows in rejected transaction");
        assert_eq!(rows, 0, "guard must write no publication rows");
    }

    #[tokio::test]
    #[serial]
    #[ignore = "requires disposable PostgreSQL and explicit real-publication guard marker"]
    async fn persist_ui_publication_rejects_typed_route_guards_before_sql() {
        assert_eq!(
            env::var("REAL_RELEASE_UI_PUBLICATION_GUARD").as_deref(),
            Ok("1"),
            "run this real-PostgreSQL test with REAL_RELEASE_UI_PUBLICATION_GUARD=1"
        );
        let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL")
            .expect("HEPHAESTUS_POSTGRES_TEST_URL is required for this real-PostgreSQL test");
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&database_url)
            .await
            .expect("connect PostgreSQL test database");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("apply release migrations");

        let parsed = parse_repository_uis(UI.as_bytes())
            .config
            .expect("valid publication fixture");
        let assistant_index = parsed
            .uis
            .iter()
            .position(|ui| ui.key.as_str() == "assistant")
            .expect("assistant UI");
        assert_eq!(parsed.uis[assistant_index].route_base.as_str(), "assistant");
        assert_eq!(
            parsed.uis[assistant_index].apis[0].route.as_str(),
            "/service/api"
        );
        let mut reserved = parsed.clone();
        let docs_index = reserved
            .uis
            .iter()
            .position(|ui| ui.key.as_str() == "docs")
            .expect("docs UI");
        reserved.uis[docs_index].route_base = UiRoutePath::parse("_heph").expect("reserved route");
        assert!(!agent_config::validate_ui_route_collisions(&reserved).is_empty());
        let mut collision = parsed;
        collision.uis[assistant_index].apis[0].route =
            RoutePath::parse("/assistant").expect("collision route");
        assert!(!agent_config::validate_ui_route_collisions(&collision).is_empty());

        for publication in [
            publication_with_config(reserved),
            publication_with_config(collision),
        ] {
            let release_id = ReleaseId::new();
            let build_request_id = BuildRequestId::new();
            let mut transaction = pool.begin().await.expect("begin publication transaction");
            let error = persist_ui_publication(
                &mut transaction,
                release_id,
                build_request_id,
                &publication,
            )
            .await
            .expect_err("typed route guard must reject before SQL");
            assert!(matches!(
                error,
                super::ReleaseServiceError::InvalidStoredData
            ));
            assert_no_publication_rows(&mut transaction, release_id).await;
            transaction
                .rollback()
                .await
                .expect("rollback rejected publication");
        }
        println!(
            "REAL_RELEASE_UI_PUBLICATION_GUARD_EXECUTED=1 cases=2 exact_error=invalid_stored_data same_transaction_zero_rows=1"
        );
        pool.close().await;
    }
}
