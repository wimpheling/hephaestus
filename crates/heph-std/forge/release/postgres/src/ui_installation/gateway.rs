use super::{GatewayRouteCandidate, UiBindingResolutionError};
use release_domain::ReleaseId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

// Keep each typed transaction and authority key explicit at this SQL boundary
// so route resolution cannot accidentally mix source and serving identities.
#[allow(clippy::too_many_arguments)]
pub(super) async fn resolve_gateway_route(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: Uuid,
    source_repository_id: Uuid,
    source_project_id: Uuid,
    release_id: ReleaseId,
    gateway_name: &str,
    release_agent_id: Uuid,
    method: &str,
    selected_route: &str,
    managed_service: bool,
) -> Result<GatewayRouteCandidate, UiBindingResolutionError> {
    let candidates: Vec<GatewayRouteCandidate> = sqlx::query_as(
        "SELECT gateway.id AS gateway_id, revision.id AS gateway_revision_id,
                gateway.name AS gateway_name,
                revision.handler_contract, route.path AS route_path,
                route.methods AS route_methods
         FROM gateways AS gateway
         JOIN gateway_revisions AS revision
           ON revision.gateway_id = gateway.id
          AND revision.id = gateway.active_revision_id
          AND revision.release_id = $3
          AND revision.release_agent_id = $4
          AND revision.exposure = 'heph_authenticated'
         JOIN gateway_routes AS route
           ON route.gateway_id = gateway.id
          AND route.gateway_revision_id = revision.id
          AND route.enabled
         WHERE gateway.repository_id = $1
           AND gateway.project_id = $2
           AND gateway.name = $5
           AND gateway.lifecycle = 'enabled'
           AND check_permission(
                 'user', $6, 'can_read', 'project', gateway.project_id::text
               ) = 1
         FOR UPDATE OF gateway",
    )
    .bind(source_repository_id)
    .bind(source_project_id)
    .bind(release_id.as_uuid())
    .bind(release_agent_id)
    .bind(gateway_name)
    .bind(actor_id.to_string())
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| UiBindingResolutionError::Persistence)?;
    candidates
        .into_iter()
        .find(|candidate| {
            (!managed_service || candidate.handler_contract == "http.service.v1")
                && (managed_service
                    || matches!(
                        candidate.handler_contract.as_str(),
                        "http.v1" | "http.service.v1"
                    ))
                && candidate.route_methods.iter().any(|value| value == method)
                && route_covers(&candidate.route_path, selected_route)
        })
        .ok_or(UiBindingResolutionError::Invalid)
}

pub(super) fn route_covers(declared: &str, selected: &str) -> bool {
    selected == declared
        || selected
            .strip_prefix(declared)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
