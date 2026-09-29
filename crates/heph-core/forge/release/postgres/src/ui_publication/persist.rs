//! Persistence of resolved release UI publication rows.

use agent_config::ui::{UiCachePolicy, UiContent};
use agent_config::validate_ui_route_collisions;
use release_domain::ui::{UiIcon, UiPresentation, UiScope};
use release_domain::{BuildRequestId, ReleaseId};
use sqlx::{Postgres, Transaction};

use super::{ReleaseServiceError, ResolvedUiPublication, invalid_ui_storage};

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
