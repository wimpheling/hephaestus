//! Route resolution and invocation recording at the gateway edge boundary.

use async_trait::async_trait;
use gateway_domain::{
    GatewayConfigRevision, GatewayEdgeError, GatewayInvocationOutcome, GatewayInvocationRecorder,
    GatewayRouteBinding, GatewayRouteResolver,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::ui_browser;
use super::{PostgresGatewayEdgeAuthority, canonical_request_path, route_matches};

pub fn desired_configuration_revision(routes: &[GatewayRouteBinding]) -> GatewayConfigRevision {
    let mut canonical = routes.to_vec();
    canonical.sort_by(|left, right| {
        left.path_prefix
            .cmp(&right.path_prefix)
            .then_with(|| left.route_id.cmp(&right.route_id))
    });
    let mut hash = Sha256::new();
    for route in canonical {
        hash.update(route.route_id.as_bytes());
        hash.update(route.gateway_revision_id.as_bytes());
        hash.update(route.path_prefix.as_bytes());
        for method in route.methods {
            hash.update(method.as_str().as_bytes());
            hash.update([0]);
        }
    }
    let digest: [u8; 32] = hash.finalize().into();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // The stable opaque UUID is only a compact carrier for the complete route
    // digest; it never becomes a source of configuration authority.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    GatewayConfigRevision::from_uuid(Uuid::from_bytes(bytes))
}

const fn outcome_name(outcome: GatewayInvocationOutcome) -> &'static str {
    match outcome {
        GatewayInvocationOutcome::Completed => "completed",
        GatewayInvocationOutcome::Failed => "failed",
        GatewayInvocationOutcome::TimedOut => "timed_out",
        GatewayInvocationOutcome::Rejected => "rejected",
    }
}

#[async_trait]
impl GatewayRouteResolver for PostgresGatewayEdgeAuthority {
    async fn resolve(
        &self,
        path_and_query: &str,
    ) -> Result<Option<GatewayRouteBinding>, GatewayEdgeError> {
        let path = canonical_request_path(path_and_query)?;
        let mut candidates = self.active_routes().await?;
        candidates.retain(|route| route_matches(route, path));
        candidates.sort_by(|left, right| {
            right
                .path_prefix
                .len()
                .cmp(&left.path_prefix.len())
                .then_with(|| left.route_id.cmp(&right.route_id))
        });
        Ok(candidates.into_iter().next())
    }
}

#[async_trait]
impl GatewayInvocationRecorder for PostgresGatewayEdgeAuthority {
    async fn accepted(
        &self,
        route: &GatewayRouteBinding,
        request_id: Uuid,
    ) -> Result<Uuid, GatewayEdgeError> {
        let invocation_id = Uuid::new_v4();
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;

        let accepted = self
            .lock_authoritative_route(&mut transaction, route)
            .await?;
        if accepted.handler_contract != "http.v1" && accepted.handler_contract != "http.service.v1"
        {
            tracing::warn!(
                invocation_id = %invocation_id,
                handler_contract = %accepted.handler_contract,
                "gateway invocation has unsupported handler contract"
            );
            return Err(GatewayEdgeError::Unavailable);
        }

        let service_binding = if accepted.handler_contract == "http.service.v1" {
            Some(
                self.service_admission_binding(&mut transaction, &accepted)
                    .await?,
            )
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO gateway_invocations
                 (id, gateway_id, gateway_revision_id, gateway_route_id,
                  project_id, request_id, outcome,
                  service_instance_id, service_instance_fencing_token)
             SELECT $1, route.gateway_id, route.gateway_revision_id, route.id,
                    route.project_id, $2, 'accepted', $5, $6
               FROM gateway_routes AS route
              WHERE route.id = $3
                AND route.gateway_revision_id = $4",
        )
        .bind(invocation_id)
        .bind(request_id)
        .bind(route.route_id)
        .bind(route.gateway_revision_id)
        .bind(service_binding.as_ref().map(|binding| binding.0))
        .bind(service_binding.as_ref().map(|binding| binding.1))
        .execute(&mut *transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;

        self.finish_accepted_invocation(invocation_id, request_id, accepted)
            .await
    }

    async fn accepted_ui(
        &self,
        route: &GatewayRouteBinding,
        authority: &gateway_domain::UiGatewayAuthority,
        request_id: Uuid,
    ) -> Result<Uuid, GatewayEdgeError> {
        ui_browser::accept_ui_invocation(self, route, authority, request_id).await
    }

    async fn completed(
        &self,
        invocation_id: Uuid,
        outcome: GatewayInvocationOutcome,
    ) -> Result<(), GatewayEdgeError> {
        let completed: bool = sqlx::query_scalar("SELECT gateway_invocation_complete($1, $2)")
            .bind(invocation_id)
            .bind(outcome_name(outcome))
            .fetch_one(&self.pool)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        if completed {
            Ok(())
        } else {
            Err(GatewayEdgeError::Unavailable)
        }
    }
}
