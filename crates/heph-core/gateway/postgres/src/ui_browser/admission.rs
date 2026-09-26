//! Public UI admission and invocation operations for the `PostgreSQL` authority.

use super::authorization::{UiAdmissionErrorExt, authorize_ui, reauthorize_after_wait};
use super::routing::{build_admission, path_component, sql_request_kind};
use crate::{AcceptedInvocationRow, PostgresGatewayEdgeAuthority};
use async_trait::async_trait;
use gateway_domain::{
    GatewayEdgeError, GatewayRouteBinding, UiGatewayAdmission, UiGatewayAdmissionError,
    UiGatewayAdmissionProvider, UiGatewayAuthority, UiGatewayRequest,
};
use uuid::Uuid;

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
