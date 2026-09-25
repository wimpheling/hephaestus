//! Worker-side `PostgreSQL` authority for the trusted UI gateway admission port.
//!
//! The adapter uses migration 0090's canonical verifier after selecting the
//! current gateway binding and reading only the immutable child digest.

use crate::{AcceptedInvocationRow, PostgresGatewayEdgeAuthority};
use async_trait::async_trait;
use gateway_domain::Exposure;
use gateway_domain::{
    GATEWAY_NAMESPACE, GatewayEdgeError, GatewayRouteBinding, UiGatewayAdmission,
    UiGatewayAdmissionError, UiGatewayAdmissionProvider, UiGatewayAuthority, UiGatewayRequest,
    UiGatewayRequestKind,
};
use http::Method;
use sqlx::{FromRow, Postgres, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, FromRow)]
struct UiGatewayBindingRow {
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
    route_id: Uuid,
    gateway_path: String,
    gateway_methods: Vec<String>,
    binding_route: String,
    route_base: String,
    handler_contract: String,
    exposure: String,
}

#[derive(Debug, FromRow)]
struct UiChildDigestRow {
    session_digest: Vec<u8>,
    installation_id: Uuid,
    generation_id: Uuid,
    organization_id: Uuid,
    route: String,
}

#[derive(Debug, FromRow)]
struct UiVerifiedSessionRow {
    session_id: Uuid,
    actor_id: Uuid,
    organization_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    route: String,
}

struct UiAuthorizedRequest {
    binding: UiGatewayBindingRow,
}

/// Implements the edge admission provider using the existing migration90
/// verifier as the canonical account, generation, permission, and binding-set
/// check. The worker reads only the digest for the safe child-session ID; no
/// raw browser secret enters this port.
pub async fn admit_ui(
    authority: &PostgresGatewayEdgeAuthority,
    request: &UiGatewayRequest,
) -> Result<UiGatewayAdmission, UiGatewayAdmissionError> {
    request
        .authority
        .validate_for(&request.method, &request.request_path_and_query)
        .map_err(|_| UiGatewayAdmissionError::Denied)?;
    let mut transaction = authority
        .pool
        .begin()
        .await
        .map_err(|_| UiGatewayAdmissionError::Unavailable)?;
    let authorized = authorize_ui(
        &mut transaction,
        &request.authority,
        &request.request_path_and_query,
        &request.method,
    )
    .await
    .map_err(UiAdmissionErrorExt::into_admission_error)?;
    let admission = build_admission(
        authority,
        &request.authority,
        &request.request_path_and_query,
        &authorized,
    )
    .map_err(|_| UiGatewayAdmissionError::Denied)?;
    transaction
        .commit()
        .await
        .map_err(|_| UiGatewayAdmissionError::Unavailable)?;
    Ok(admission)
}

/// Inserts a UI invocation only after the same transaction has locked the
/// selected gateway route, loaded the child digest, invoked the canonical
/// verifier, and compared every safe identity. Service routes retain the
/// existing readiness, lease, owner, and fencing checks.
// Keep the lock, canonical recheck, and linearizing insert together so the
// authorization-to-invocation sequence remains auditable as one transaction.
#[allow(clippy::too_many_lines)]
pub async fn accept_ui_invocation(
    authority: &PostgresGatewayEdgeAuthority,
    route: &GatewayRouteBinding,
    safe_authority: &UiGatewayAuthority,
    request_id: Uuid,
) -> Result<Uuid, GatewayEdgeError> {
    safe_authority.validate_for(
        &safe_authority.method,
        &safe_authority.canonical_request_path,
    )?;
    let invocation_id = Uuid::new_v4();
    let mut transaction = authority
        .pool
        .begin()
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
    let authorized = authorize_ui(
        &mut transaction,
        safe_authority,
        &safe_authority.canonical_request_path,
        &safe_authority.method,
    )
    .await?;
    let accepted_gateway_id = authorized.binding.gateway_id;
    let accepted_revision_id = authorized.binding.gateway_revision_id;
    let accepted_contract = authorized.binding.handler_contract.clone();
    let admission = build_admission(
        authority,
        safe_authority,
        &safe_authority.canonical_request_path,
        &authorized,
    )
    .map_err(|_| GatewayEdgeError::Unavailable)?;
    if admission.route != *route {
        return Err(GatewayEdgeError::Unavailable);
    }
    let accepted = AcceptedInvocationRow {
        gateway_id: accepted_gateway_id,
        gateway_revision_id: accepted_revision_id,
        handler_contract: accepted_contract,
    };
    let service_binding = if accepted.handler_contract == "http.service.v1" {
        Some(
            authority
                .service_admission_binding(&mut transaction, &accepted)
                .await?,
        )
    } else {
        None
    };
    reauthorize_after_wait(
        authority,
        &mut transaction,
        safe_authority,
        route,
        &accepted,
    )
    .await?;
    let inserted = sqlx::query(
        "INSERT INTO gateway_invocations
             (id, gateway_id, gateway_revision_id, gateway_route_id,
              project_id, request_id, outcome,
              service_instance_id, service_instance_fencing_token)
         SELECT $1, route.gateway_id, route.gateway_revision_id, route.id,
                route.project_id, $2, 'accepted', $5, $6
           FROM gateway_routes AS route
           JOIN ui_browser_sessions AS child
             ON child.id = $7
           CROSS JOIN LATERAL authenticate_ui_browser_session(
               child.session_digest, $8, $9, $10, $11
           ) AS verified
          WHERE route.id = $3
            AND route.gateway_revision_id = $4
            AND route.enabled
            AND $11 = ANY(route.methods)
            AND child.installation_id = $12
            AND child.generation_id = $8
            AND child.organization_id = $13
            AND verified.session_id = child.id
            AND verified.actor_id = $14
            AND verified.organization_id = $13
            AND verified.installation_id = $12
            AND verified.generation_id = $8
            AND verified.route = child.route
            AND (
                $5::uuid IS NULL
                OR EXISTS (
                    SELECT 1
                    FROM gateway_service_instances AS instance
                    WHERE instance.id = $5
                      AND instance.fencing_token = $6
                      AND instance.state = 'ready'
                      AND instance.lease_expires_at > clock_timestamp()
                )
            )",
    )
    .bind(invocation_id)
    .bind(request_id)
    .bind(route.route_id)
    .bind(route.gateway_revision_id)
    .bind(service_binding.as_ref().map(|binding| binding.0))
    .bind(service_binding.as_ref().map(|binding| binding.1))
    .bind(safe_authority.child_session_id)
    .bind(safe_authority.generation_id)
    .bind(sql_request_kind(safe_authority.request_kind))
    .bind(path_component(&safe_authority.canonical_request_path))
    .bind(safe_authority.method.as_str())
    .bind(safe_authority.installation_id)
    .bind(safe_authority.organization_id)
    .bind(safe_authority.actor_id)
    .execute(&mut *transaction)
    .await
    .map_err(|_| GatewayEdgeError::Unavailable)?;
    if inserted.rows_affected() != 1 {
        return Err(GatewayEdgeError::Unavailable);
    }
    transaction
        .commit()
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
    authority
        .finish_accepted_invocation(invocation_id, request_id, accepted)
        .await
}

#[async_trait]
impl UiGatewayAdmissionProvider for PostgresGatewayEdgeAuthority {
    async fn admit(
        &self,
        request: &UiGatewayRequest,
    ) -> Result<UiGatewayAdmission, UiGatewayAdmissionError> {
        admit_ui(self, request).await
    }
}

// The verifier's session ID is intentionally compared with the authority's
// child session ID; the authority has no separate parent-session field.
#[allow(clippy::suspicious_operation_groupings)]
async fn authorize_ui(
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

async fn reauthorize_after_wait(
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

async fn lock_ui_binding(
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

fn build_admission(
    authority: &PostgresGatewayEdgeAuthority,
    safe_authority: &UiGatewayAuthority,
    request_path_and_query: &str,
    authorized: &UiAuthorizedRequest,
) -> Result<UiGatewayAdmission, GatewayEdgeError> {
    if authorized.binding.exposure != "heph_authenticated"
        || (authorized.binding.handler_contract != "http.v1"
            && authorized.binding.handler_contract != "http.service.v1")
    {
        return Err(GatewayEdgeError::Unavailable);
    }
    let path = path_component(request_path_and_query);
    let gateway_path = match safe_authority.request_kind {
        UiGatewayRequestKind::Managed => {
            let suffix = path
                .strip_prefix(&authorized.binding.route_base)
                .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'))
                .ok_or(GatewayEdgeError::Unavailable)?;
            format!("{}{}", authorized.binding.binding_route, suffix)
        }
        UiGatewayRequestKind::Api => {
            if path != authorized.binding.binding_route {
                return Err(GatewayEdgeError::Unavailable);
            }
            authorized.binding.binding_route.clone()
        }
    };
    let methods = authorized
        .binding
        .gateway_methods
        .iter()
        .map(|value| persisted_method(value))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let path_prefix = authorized
        .binding
        .gateway_path
        .strip_prefix('/')
        .ok_or(GatewayEdgeError::Unavailable)?
        .to_owned();
    let route = GatewayRouteBinding {
        route_id: authorized.binding.route_id,
        gateway_revision_id: authorized.binding.gateway_revision_id,
        exposure: Exposure::HephAuthenticated,
        path_prefix,
        methods,
        limits: authority.limits,
    };
    route.validate()?;
    let query = request_path_and_query
        .split_once('?')
        .map(|(_, query)| query);
    let gateway_path = format!(
        "{GATEWAY_NAMESPACE}{}",
        gateway_path.trim_start_matches('/')
    );
    let gateway_path_and_query = match query {
        Some(query) => format!("{gateway_path}?{query}"),
        None => gateway_path,
    };
    Ok(UiGatewayAdmission {
        route,
        gateway_path_and_query,
    })
}

fn path_component(path_and_query: &str) -> &str {
    path_and_query
        .split_once('?')
        .map_or(path_and_query, |(path, _)| path)
}

const fn sql_request_kind(kind: UiGatewayRequestKind) -> &'static str {
    match kind {
        UiGatewayRequestKind::Managed => "managed_service",
        UiGatewayRequestKind::Api => "api",
    }
}

fn persisted_method(value: &str) -> Result<Method, GatewayEdgeError> {
    match value {
        "GET" => Ok(Method::GET),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        "HEAD" => Ok(Method::HEAD),
        "OPTIONS" => Ok(Method::OPTIONS),
        _ => Err(GatewayEdgeError::Unavailable),
    }
}

trait UiAdmissionErrorExt {
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
