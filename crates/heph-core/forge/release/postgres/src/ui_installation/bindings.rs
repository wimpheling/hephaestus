use super::gateway::resolve_gateway_route;
use super::{ResolvedUiBinding, UiBindingResolutionError};
use release_domain::ReleaseId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) async fn resolve_repository_git_access(
    tx: &mut Transaction<'_, Postgres>,
    release_id: ReleaseId,
    ui_key: &release_domain::ui::UiKey,
    target_scope: &str,
    repository_target: bool,
    acknowledged: bool,
    previous_generation_id: Option<Uuid>,
) -> Result<&'static str, UiBindingResolutionError> {
    let access: String = sqlx::query_scalar(
        "SELECT repository_git_access
         FROM release_ui_descriptors
         WHERE release_id = $1 AND ui_key = $2 AND scope = $3",
    )
    .bind(release_id.as_uuid())
    .bind(ui_key.as_str())
    .bind(target_scope)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| UiBindingResolutionError::Persistence)?
    .ok_or(UiBindingResolutionError::Invalid)?;
    let access = release_domain::ui::UiRepositoryGitAccess::parse(access)
        .map_err(|_| UiBindingResolutionError::Invalid)?;
    if !repository_target && !matches!(access, release_domain::ui::UiRepositoryGitAccess::None) {
        return Err(UiBindingResolutionError::Invalid);
    }
    if matches!(access, release_domain::ui::UiRepositoryGitAccess::None) {
        return Ok(access.as_str());
    }
    let approved = if acknowledged {
        true
    } else if let Some(previous_generation_id) = previous_generation_id {
        let previous: String = sqlx::query_scalar(
            "SELECT repository_git_access
             FROM ui_installation_generations WHERE id = $1",
        )
        .bind(previous_generation_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| UiBindingResolutionError::Persistence)?
        .ok_or(UiBindingResolutionError::Invalid)?;
        git_access_rank(
            release_domain::ui::UiRepositoryGitAccess::parse(previous)
                .map_err(|_| UiBindingResolutionError::Invalid)?,
        ) >= git_access_rank(access)
    } else {
        false
    };
    if approved {
        Ok(access.as_str())
    } else {
        Err(UiBindingResolutionError::Invalid)
    }
}
pub(super) const fn git_access_rank(access: release_domain::ui::UiRepositoryGitAccess) -> u8 {
    match access {
        release_domain::ui::UiRepositoryGitAccess::None => 0,
        release_domain::ui::UiRepositoryGitAccess::Read => 1,
        release_domain::ui::UiRepositoryGitAccess::ReadWrite => 2,
    }
}

// Keep target-shape, gateway, and binding validation together so every
// installation scope follows the same publication and authorization path.
#[allow(clippy::too_many_lines)]
pub(super) async fn resolve_ui_bindings(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: Uuid,
    release_id: ReleaseId,
    target_organization_id: Uuid,
    target_scope: &str,
    ui_key: &release_domain::ui::UiKey,
    static_zero_api_only: bool,
) -> Result<Vec<ResolvedUiBinding>, UiBindingResolutionError> {
    let publication: Option<(String, String, Uuid, Uuid)> = sqlx::query_as(
        "SELECT descriptor.scope, descriptor.content_kind,
                source_repository.id, source_project.id
         FROM release_ui_descriptors AS descriptor
         JOIN releases AS release ON release.id = descriptor.release_id
         JOIN repositories AS source_repository
           ON source_repository.id = release.repository_id
         JOIN projects AS source_project
           ON source_project.id = source_repository.project_id
         WHERE descriptor.release_id = $1
           AND descriptor.ui_key = $2
           AND descriptor.scope = $3
           AND source_project.organization_id = $4
           AND release.state = 'published'
         FOR SHARE OF release, source_repository, source_project",
    )
    .bind(release_id.as_uuid())
    .bind(ui_key.as_str())
    .bind(target_scope)
    .bind(target_organization_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| UiBindingResolutionError::Persistence)?;
    let Some((scope, content_kind, source_repository_id, source_project_id)) = publication else {
        return Err(UiBindingResolutionError::Invalid);
    };
    if scope != target_scope {
        return Err(UiBindingResolutionError::Invalid);
    }

    let managed: Option<(String, String, Uuid)> = sqlx::query_as(
        "SELECT gateway_name, route, release_agent_id
         FROM release_ui_managed_services
         WHERE release_id = $1 AND ui_key = $2",
    )
    .bind(release_id.as_uuid())
    .bind(ui_key.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| UiBindingResolutionError::Persistence)?;
    let apis: Vec<(String, String, String, String, Uuid)> = sqlx::query_as(
        "SELECT api_key, gateway_name, method, route, release_agent_id
         FROM release_ui_api_bindings
         WHERE release_id = $1 AND ui_key = $2
         ORDER BY api_key",
    )
    .bind(release_id.as_uuid())
    .bind(ui_key.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| UiBindingResolutionError::Persistence)?;

    if static_zero_api_only && (content_kind != "static" || managed.is_some() || !apis.is_empty()) {
        return Err(UiBindingResolutionError::Invalid);
    }
    if content_kind == "static" {
        let has_file: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM release_ui_static_files
                 WHERE release_id = $1 AND ui_key = $2
             )",
        )
        .bind(release_id.as_uuid())
        .bind(ui_key.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| UiBindingResolutionError::Persistence)?;
        if !has_file || managed.is_some() {
            return Err(UiBindingResolutionError::Invalid);
        }
    } else if content_kind == "managed_service" {
        if managed.is_none() {
            return Err(UiBindingResolutionError::Invalid);
        }
    } else {
        return Err(UiBindingResolutionError::Invalid);
    }

    let mut bindings = Vec::with_capacity(usize::from(managed.is_some()) + apis.len());
    if let Some((gateway_name, route, release_agent_id)) = managed {
        let candidate = resolve_gateway_route(
            tx,
            actor_id,
            source_repository_id,
            source_project_id,
            release_id,
            &gateway_name,
            release_agent_id,
            "GET",
            &route,
            true,
        )
        .await?;
        bindings.push(ResolvedUiBinding {
            binding_kind: "managed_service",
            binding_key: String::from("service"),
            gateway_id: candidate.gateway_id,
            gateway_revision_id: candidate.gateway_revision_id,
            release_agent_id,
            gateway_name: candidate.gateway_name,
            method: String::from("GET"),
            route,
        });
    }
    for (binding_key, gateway_name, method, route, release_agent_id) in apis {
        let candidate = resolve_gateway_route(
            tx,
            actor_id,
            source_repository_id,
            source_project_id,
            release_id,
            &gateway_name,
            release_agent_id,
            &method,
            &route,
            false,
        )
        .await?;
        bindings.push(ResolvedUiBinding {
            binding_kind: "api",
            binding_key,
            gateway_id: candidate.gateway_id,
            gateway_revision_id: candidate.gateway_revision_id,
            release_agent_id,
            gateway_name: candidate.gateway_name,
            method,
            route,
        });
    }
    Ok(bindings)
}
