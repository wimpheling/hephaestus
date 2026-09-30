//! UI gateway binding and canonical browser-session authorization.

use super::routing::{build_admission, path_component, sql_request_kind};
use crate::{AcceptedInvocationRow, PostgresGatewayEdgeAuthority};
use gateway_domain::{
    GatewayEdgeError, GatewayRouteBinding, UiGatewayAdmissionError, UiGatewayAuthority,
    UiGatewayRequestKind,
};
use http::Method;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub(super) struct UiGatewayBindingRow {
    pub(super) gateway_id: Uuid,
    pub(super) gateway_revision_id: Uuid,
    pub(super) route_id: Uuid,
    pub(super) gateway_path: String,
    pub(super) gateway_methods: Vec<String>,
    pub(super) binding_route: String,
    pub(super) route_base: String,
    pub(super) handler_contract: String,
    pub(super) exposure: String,
}

#[derive(Debug, FromRow)]
pub(super) struct UiChildDigestRow {
    pub(super) session_digest: Vec<u8>,
    pub(super) installation_id: Uuid,
    pub(super) generation_id: Uuid,
    pub(super) organization_id: Uuid,
    pub(super) route: String,
}

#[derive(Debug, FromRow)]
pub(super) struct UiVerifiedSessionRow {
    pub(super) session_id: Uuid,
    pub(super) actor_id: Uuid,
    pub(super) organization_id: Uuid,
    pub(super) installation_id: Uuid,
    pub(super) generation_id: Uuid,
    pub(super) route: String,
}

pub(super) struct UiAuthorizedRequest {
    pub(super) binding: UiGatewayBindingRow,
}

// The verifier's session ID is intentionally compared with the authority's
// child session ID; the authority has no separate parent-session field.
#[allow(clippy::suspicious_operation_groupings)]
pub(super) async fn authorize_ui(
    transaction: &mut Transaction<'_, Postgres>,
    authority: &UiGatewayAuthority,
    request_path_and_query: &str,
    method: &Method,
) -> Result<UiAuthorizedRequest, GatewayEdgeError> {
    let path = path_component(request_path_and_query);
    let binding =
        lock_ui_binding(transaction, authority, authority.request_kind, path, method).await?;
    let child = sqlx::query_as::<_, UiChildDigestRow>(
        "SELECT session_digest, installation_id, generation_id,
                organization_id, route
           FROM ui_browser_sessions
          WHERE id = $1
            AND installation_id = $2
            AND generation_id = $3",
    )
    .bind(authority.child_session_id)
    .bind(authority.installation_id)
    .bind(authority.generation_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| GatewayEdgeError::Unavailable)?
    .ok_or(GatewayEdgeError::Contract("UI authority denied"))?;
    let request_kind = sql_request_kind(authority.request_kind);
    let verified = sqlx::query_as::<_, UiVerifiedSessionRow>(
        "SELECT session_id, actor_id, organization_id,
                installation_id, generation_id, route
           FROM authenticate_ui_browser_session($1, $2, $3, $4, $5)",
    )
    .bind(child.session_digest)
    .bind(authority.generation_id)
    .bind(request_kind)
    .bind(path)
    .bind(method.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| GatewayEdgeError::Unavailable)?
    .ok_or(GatewayEdgeError::Contract("UI authority denied"))?;
    let verified_identity_matches = verified.session_id == authority.child_session_id
        && verified.actor_id == authority.actor_id
        && verified.organization_id == authority.organization_id
        && verified.installation_id == authority.installation_id
        && verified.generation_id == authority.generation_id
        && verified.route == child.route;
    let child_identity_matches = child.organization_id == authority.organization_id
        && child.installation_id == authority.installation_id
        && child.generation_id == authority.generation_id;
    if !verified_identity_matches || !child_identity_matches {
        return Err(GatewayEdgeError::Unavailable);
    }
    Ok(UiAuthorizedRequest { binding })
}

pub(super) async fn reauthorize_after_wait(
    authority: &PostgresGatewayEdgeAuthority,
    transaction: &mut Transaction<'_, Postgres>,
    safe_authority: &UiGatewayAuthority,
    route: &GatewayRouteBinding,
    accepted: &AcceptedInvocationRow,
) -> Result<(), GatewayEdgeError> {
    // Service admission may wait on both the instance and publication locks.
    // Re-run migration 0090's canonical verifier after those waits so a
    // parent or child revoked during the wait cannot reach the insert.
    let reauthorized = authorize_ui(
        transaction,
        safe_authority,
        &safe_authority.canonical_request_path,
        &safe_authority.method,
    )
    .await?;
    let reauthorized_admission = build_admission(
        authority,
        safe_authority,
        &safe_authority.canonical_request_path,
        &reauthorized,
    )
    .map_err(|_| GatewayEdgeError::Unavailable)?;
    if reauthorized_admission.route != *route
        || reauthorized.binding.gateway_id != accepted.gateway_id
        || reauthorized.binding.gateway_revision_id != accepted.gateway_revision_id
        || reauthorized.binding.handler_contract != accepted.handler_contract
    {
        return Err(GatewayEdgeError::Unavailable);
    }
    Ok(())
}

pub(super) async fn lock_ui_binding(
    transaction: &mut Transaction<'_, Postgres>,
    authority: &UiGatewayAuthority,
    request_kind: UiGatewayRequestKind,
    path: &str,
    method: &Method,
) -> Result<UiGatewayBindingRow, GatewayEdgeError> {
    let kind = sql_request_kind(request_kind);
    sqlx::query_as::<_, UiGatewayBindingRow>(
        "SELECT binding.gateway_id,
                binding.gateway_revision_id,
                gateway_route.id AS route_id,
                gateway_route.path AS gateway_path,
                gateway_route.methods AS gateway_methods,
                binding.route AS binding_route,
                descriptor.route_base,
                revision.handler_contract,
                revision.exposure
           FROM ui_installation_bindings AS binding
           JOIN ui_installation_generations AS generation
             ON generation.id = binding.generation_id
            AND generation.installation_id = binding.installation_id
            AND generation.release_id = binding.release_id
            AND generation.ui_key = binding.ui_key
           JOIN release_ui_descriptors AS descriptor
             ON descriptor.release_id = binding.release_id
            AND descriptor.ui_key = binding.ui_key
            AND descriptor.scope = generation.ui_scope
           JOIN gateways AS gateway
             ON gateway.id = binding.gateway_id
            AND gateway.lifecycle = 'enabled'
            AND gateway.active_revision_id = binding.gateway_revision_id
           JOIN gateway_revisions AS revision
             ON revision.id = binding.gateway_revision_id
            AND revision.gateway_id = binding.gateway_id
           JOIN gateway_routes AS gateway_route
             ON gateway_route.gateway_id = gateway.id
            AND gateway_route.gateway_revision_id = revision.id
            AND gateway_route.enabled
            AND (
                gateway_route.path = binding.route
                OR (
                    length(binding.route) > length(gateway_route.path)
                    AND left(binding.route, length(gateway_route.path) + 1)
                        = gateway_route.path || '/'
                )
            )
            AND $4 = ANY(gateway_route.methods)
          WHERE binding.installation_id = $1
            AND binding.generation_id = $2
            AND binding.binding_kind = $3
            AND binding.method = $4
            AND (
                binding.binding_kind = 'managed_service'
                OR binding.route = $5
            )
          ORDER BY length(gateway_route.path) DESC, gateway_route.id
          LIMIT 1
          FOR UPDATE OF gateway, gateway_route",
    )
    .bind(authority.installation_id)
    .bind(authority.generation_id)
    .bind(kind)
    .bind(method.as_str())
    .bind(path)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| GatewayEdgeError::Unavailable)?
    .ok_or(GatewayEdgeError::Contract("UI route is not admitted"))
}

pub(super) trait UiAdmissionErrorExt {
    fn into_admission_error(self) -> UiGatewayAdmissionError;
}

impl UiAdmissionErrorExt for GatewayEdgeError {
    fn into_admission_error(self) -> UiGatewayAdmissionError {
        match self {
            Self::Contract("UI route is not admitted") => UiGatewayAdmissionError::NotFound,
            Self::Unavailable | Self::HandlerUnavailable => UiGatewayAdmissionError::Unavailable,
            _ => UiGatewayAdmissionError::Denied,
        }
    }
}
