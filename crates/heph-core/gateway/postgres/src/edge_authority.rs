//! Gateway edge authority lifecycle and post-admission setup.

use gateway_domain::{
    GatewayDesiredConfiguration, GatewayEdgeError, GatewayInvocationOutcome,
    GatewayInvocationRecorder, GatewayLimits,
};
use runtime_authority::{GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    edge_invocations::desired_configuration_revision, edge_recovery,
    route_authority::AcceptedInvocationRow,
};

/// Private worker adapter from authoritative gateway rows to the edge ports.
///
/// It never trusts a route identifier supplied by Caddy: resolution starts with
/// the canonical request path and selects only an enabled route of the active
/// immutable revision. Invocation rows retain correlation/lifecycle evidence
/// only; payloads remain at the private HTTP boundary.
#[derive(Clone)]
pub struct PostgresGatewayEdgeAuthority {
    pub(super) pool: PgPool,
    pub(super) limits: GatewayLimits,
    pub(super) runtime_authority: Option<Arc<dyn GatewayRuntimeAuthorityIssuer>>,
    pub(super) session_ttl: Duration,
}

impl PostgresGatewayEdgeAuthority {
    /// Creates the worker-side route and invocation adapter with explicit
    /// bounded HTTP limits.
    #[must_use]
    pub const fn new(pool: PgPool, limits: GatewayLimits) -> Self {
        Self {
            pool,
            limits,
            runtime_authority: None,
            session_ttl: Duration::from_secs(30),
        }
    }

    /// Attaches the generic gateway-session issuer used by a production
    /// dispatcher. The basic constructor remains useful for reconciliation
    /// workers that never accept public traffic.
    ///
    /// # Errors
    ///
    /// Returns an unavailable edge error when the requested session lifetime
    /// is zero and therefore cannot form a bounded session identity.
    pub fn with_runtime_authority(
        mut self,
        runtime_authority: Arc<dyn GatewayRuntimeAuthorityIssuer>,
        session_ttl: Duration,
    ) -> Result<Self, GatewayEdgeError> {
        if session_ttl.is_zero() {
            return Err(GatewayEdgeError::Unavailable);
        }
        self.runtime_authority = Some(runtime_authority);
        self.session_ttl = session_ttl;
        Ok(self)
    }

    /// Reconstructs the complete enabled desired route set from `PostgreSQL`.
    /// The returned revision is a deterministic digest of the authoritative
    /// active route set; no provider configuration becomes authoritative.
    ///
    /// # Errors
    ///
    /// Returns an unavailable edge error when authoritative rows cannot be
    /// read or contain an invalid persisted HTTP vocabulary.
    pub async fn desired_configuration(
        &self,
    ) -> Result<GatewayDesiredConfiguration, GatewayEdgeError> {
        let routes = self.active_routes().await?;
        Ok(GatewayDesiredConfiguration {
            revision: desired_configuration_revision(&routes),
            routes,
        })
    }

    async fn reject_invocation(&self, invocation_id: Uuid) -> Result<(), GatewayEdgeError> {
        let _: bool = sqlx::query_scalar("SELECT gateway_invocation_complete($1, 'rejected')")
            .bind(invocation_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        Ok(())
    }

    async fn create_gateway_secret_leases(
        &self,
        invocation_id: Uuid,
        runtime_session_id: Uuid,
    ) -> Result<(), GatewayEdgeError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        let invocation: Option<(Uuid, Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT gateway_id, gateway_revision_id, gateway_route_id, outcome
               FROM gateway_invocations
              WHERE id = $1
              FOR UPDATE",
        )
        .bind(invocation_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let Some((gateway_id, revision_id, route_id, outcome)) = invocation else {
            return Err(GatewayEdgeError::Unavailable);
        };
        if outcome != "accepted" {
            return Err(GatewayEdgeError::Unavailable);
        }
        let session: Option<(Uuid, Uuid, Uuid, String, OffsetDateTime)> = sqlx::query_as(
            "SELECT id, invocation_id, gateway_revision_id, status, expires_at
               FROM gateway_runtime_authority_sessions
              WHERE id = $1
                AND invocation_id = $2
                AND gateway_id = $3
                AND gateway_revision_id = $4
                AND status IN ('pending_handoff', 'active')
                AND expires_at > now()
              FOR UPDATE",
        )
        .bind(runtime_session_id)
        .bind(invocation_id)
        .bind(gateway_id)
        .bind(revision_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let Some((session_id, session_invocation_id, session_revision_id, _status, expires_at)) =
            session
        else {
            return Err(GatewayEdgeError::Unavailable);
        };
        // The invocation row is locked before the session row, matching
        // terminal completion and repository lifecycle cleanup.
        sqlx::query(
            "INSERT INTO gateway_secret_leases
                 (id, runtime_session_id, invocation_id, binding_id,
                  secret_version_id, rule_id, status, expires_at)
             SELECT gen_random_uuid(), $1, $2, binding.id,
                    binding.secret_version_id, rule.id, 'active', $3
             FROM gateway_brokered_secret_rules AS rule
             JOIN gateway_secret_bindings AS binding
               ON binding.id = rule.binding_id
              AND binding.gateway_revision_id = $5
             WHERE rule.gateway_revision_id = $5
               AND rule.gateway_route_id = $4
               AND binding.status = 'active'
             ON CONFLICT (invocation_id, rule_id) DO NOTHING",
        )
        .bind(session_id)
        .bind(session_invocation_id)
        .bind(expires_at)
        .bind(route_id)
        .bind(session_revision_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)
    }

    /// Terminalizes one bounded batch of abandoned host-mediated service
    /// invocations. The caller owns scheduling repeated batches.
    ///
    /// # Errors
    ///
    /// Returns an unavailable error when the authoritative recovery query or
    /// terminal transition cannot be completed.
    pub async fn recover_abandoned_service_invocations(
        &self,
        now: OffsetDateTime,
    ) -> Result<u64, GatewayEdgeError> {
        let Ok(ttl) = time::Duration::try_from(self.session_ttl) else {
            return Err(GatewayEdgeError::Unavailable);
        };
        let Some(cutoff) = now.checked_sub(ttl) else {
            return Ok(0);
        };
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        let candidates = edge_recovery::recovery_candidates(&mut transaction, cutoff, now).await?;
        let mut processed = 0;
        for invocation_id in candidates {
            let eligible = edge_recovery::recovery_candidate_is_eligible(
                &mut transaction,
                invocation_id,
                now,
                cutoff,
            )
            .await?;
            if !eligible {
                continue;
            }
            let changed: bool =
                sqlx::query_scalar("SELECT gateway_invocation_complete($1, 'timed_out')")
                    .bind(invocation_id)
                    .fetch_one(&mut *transaction)
                    .await
                    .map_err(|_| GatewayEdgeError::Unavailable)?;
            if changed {
                processed += 1;
            }
        }
        transaction
            .commit()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        Ok(processed)
    }
    pub(super) async fn finish_accepted_invocation(
        &self,
        invocation_id: Uuid,
        request_id: Uuid,
        accepted: AcceptedInvocationRow,
    ) -> Result<Uuid, GatewayEdgeError> {
        let Some(issuer) = &self.runtime_authority else {
            if accepted.handler_contract == "http.service.v1" {
                warn_post_admission_setup(request_id, invocation_id, "runtime_authority_missing");
                self.reject_invocation(invocation_id).await?;
                return Err(GatewayEdgeError::Unavailable);
            }
            return Ok(invocation_id);
        };
        let issued_at = OffsetDateTime::now_utc();
        let Ok(ttl) = time::Duration::try_from(self.session_ttl) else {
            self.reject_invocation(invocation_id).await?;
            return Err(GatewayEdgeError::Unavailable);
        };
        let Some(expires_at) = issued_at.checked_add(ttl) else {
            self.reject_invocation(invocation_id).await?;
            return Err(GatewayEdgeError::Unavailable);
        };
        let request = GatewayRuntimeSessionRequest {
            invocation_id: capability_domain::GatewayInvocationId::from_uuid(invocation_id),
            gateway_id: accepted.gateway_id,
            gateway_revision_id: accepted.gateway_revision_id,
            issued_at,
            expires_at,
        };
        let issued_result = if accepted.handler_contract == "http.service.v1" {
            issuer.issue_gateway_service(request).await
        } else {
            issuer.issue_gateway(request).await
        };
        let Ok(issued_session) = issued_result else {
            warn_post_admission_setup(
                request_id,
                invocation_id,
                "runtime_authority_issuance_failed",
            );
            self.reject_invocation(invocation_id).await?;
            return Err(GatewayEdgeError::Unavailable);
        };
        if let Err(error) = self
            .create_gateway_secret_leases(invocation_id, issued_session.id.as_uuid())
            .await
        {
            warn_post_admission_setup(request_id, invocation_id, "secret_lease_setup_failed");
            let _ = self
                .completed(invocation_id, GatewayInvocationOutcome::Rejected)
                .await;
            return Err(error);
        }
        Ok(invocation_id)
    }
}

fn warn_post_admission_setup(request_id: Uuid, invocation_id: Uuid, category: &'static str) {
    tracing::warn!(
        target: "gateway_postgres::ui_post_admission",
        request_id = %request_id,
        invocation_id = %invocation_id,
        category,
        "gateway post-admission setup failed"
    );
}
