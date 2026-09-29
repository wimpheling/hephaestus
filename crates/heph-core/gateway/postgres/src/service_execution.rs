//! `PostgreSQL` execution-target authority for accepted gateway invocations.

use async_trait::async_trait;
use gateway_domain::{
    GatewayExecutionTarget, GatewayExecutionTargetError, GatewayExecutionTargetResolver,
    GatewayServiceAuthorityBudget, GatewayServiceInstanceKey, GatewayServiceOwner,
};
use sqlx::{FromRow, PgPool};
use std::{
    convert::TryFrom,
    time::{Duration, Instant},
};
use time::{Duration as TimeDuration, OffsetDateTime};
use uuid::Uuid;

/// Worker-role adapter for immutable gateway invocation execution targets.
#[derive(Clone)]
pub struct PostgresGatewayExecutionTargetResolver {
    pool: PgPool,
}

impl PostgresGatewayExecutionTargetResolver {
    /// Creates a resolver over the gateway worker pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GatewayExecutionTargetResolver for PostgresGatewayExecutionTargetResolver {
    async fn resolve_execution_target(
        &self,
        invocation_id: Uuid,
        route_id: Uuid,
        revision_id: Uuid,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayExecutionTarget, GatewayExecutionTargetError> {
        if invocation_id.is_nil() || route_id.is_nil() || revision_id.is_nil() {
            return Err(GatewayExecutionTargetError::InvalidArgument);
        }
        owner
            .validate()
            .map_err(|_| GatewayExecutionTargetError::InvalidArgument)?;
        let started = Instant::now();
        let row =
            load_execution_target(&self.pool, invocation_id, route_id, revision_id, owner).await?;

        let Some(row) = row else {
            return Err(GatewayExecutionTargetError::Unavailable);
        };
        if row.handler_contract == "http.v1" {
            return Ok(GatewayExecutionTarget::Stateless);
        }
        if row.handler_contract != "http.service.v1" {
            return Err(GatewayExecutionTargetError::Unavailable);
        }
        let (Some(instance_id), Some(fencing_token), Some(session_expires_at)) = (
            row.service_instance_id,
            row.service_instance_fencing_token,
            row.session_expires_at,
        ) else {
            return Err(GatewayExecutionTargetError::Unavailable);
        };
        let remaining = session_expires_at - row.database_now;
        let elapsed = TimeDuration::try_from(started.elapsed())
            .map_err(|_| GatewayExecutionTargetError::Unavailable)?;
        let remaining = remaining
            .checked_sub(elapsed)
            .filter(|duration| duration.is_positive())
            .ok_or(GatewayExecutionTargetError::Unavailable)?;
        let remaining =
            Duration::try_from(remaining).map_err(|_| GatewayExecutionTargetError::Unavailable)?;
        if remaining.is_zero() {
            return Err(GatewayExecutionTargetError::Unavailable);
        }
        Ok(GatewayExecutionTarget::Service(
            GatewayServiceAuthorityBudget {
                instance: GatewayServiceInstanceKey {
                    identity: gateway_domain::GatewayServiceIdentity {
                        instance_id,
                        gateway_id: row.gateway_id,
                        revision_id: row.gateway_revision_id,
                    },
                    fencing_token,
                },
                expires_at: session_expires_at,
                remaining,
            },
        ))
    }
}

async fn load_execution_target(
    pool: &PgPool,
    invocation_id: Uuid,
    route_id: Uuid,
    revision_id: Uuid,
    owner: &GatewayServiceOwner,
) -> Result<Option<ExecutionTargetRow>, GatewayExecutionTargetError> {
    sqlx::query_as::<_, ExecutionTargetRow>(
        r"
SELECT invocation.gateway_id,
       invocation.gateway_revision_id,
       revision.handler_contract,
       invocation.service_instance_id,
       invocation.service_instance_fencing_token,
       session.expires_at AS session_expires_at,
       clock.database_now
  FROM gateway_invocations AS invocation
  JOIN gateway_routes AS route
    ON route.id = invocation.gateway_route_id
   AND route.gateway_revision_id = invocation.gateway_revision_id
   AND route.gateway_id = invocation.gateway_id
  JOIN gateways AS gateway
    ON gateway.id = invocation.gateway_id
   AND gateway.id = route.gateway_id
  JOIN gateway_revisions AS revision
    ON revision.id = invocation.gateway_revision_id
   AND revision.gateway_id = invocation.gateway_id
  CROSS JOIN LATERAL (
       SELECT clock_timestamp() AS database_now
  ) AS clock
  LEFT JOIN gateway_service_instances AS instance
    ON instance.id = invocation.service_instance_id
   AND instance.gateway_id = invocation.gateway_id
   AND instance.revision_id = invocation.gateway_revision_id
   AND instance.fencing_token = invocation.service_instance_fencing_token
   AND instance.owner_host_id = $4
   AND instance.owner_uuid = $5
   AND instance.state IN ('ready', 'draining')
   AND instance.lease_expires_at > clock.database_now
  LEFT JOIN releases AS release
    ON release.id = revision.release_id
   AND release.repository_id = revision.repository_id
   AND release.state = 'published'
  LEFT JOIN gateway_runtime_authority_sessions AS session
    ON session.invocation_id = invocation.id
   AND session.gateway_id = invocation.gateway_id
   AND session.gateway_revision_id = invocation.gateway_revision_id
   AND session.admission_mode = 'host_mediated'
   AND session.status = 'active'
   AND session.expires_at > clock.database_now
 WHERE invocation.id = $1
   AND invocation.gateway_route_id = $2
   AND invocation.gateway_revision_id = $3
   AND invocation.outcome = 'accepted'
   AND (
       (
           revision.handler_contract = 'http.v1'
           AND invocation.service_instance_id IS NULL
           AND invocation.service_instance_fencing_token IS NULL
       )
       OR
       (
           revision.handler_contract = 'http.service.v1'
           AND instance.id IS NOT NULL
           AND release.id IS NOT NULL
           AND session.id IS NOT NULL
           AND NOT EXISTS (
               SELECT 1
                 FROM gateway_secret_leases AS lease
                 JOIN gateway_brokered_secret_rules AS rule
                   ON rule.id = lease.rule_id
                  AND rule.gateway_route_id = route.id
                  AND rule.gateway_revision_id = revision.id
                WHERE lease.invocation_id = invocation.id
                  AND (lease.status <> 'active'
                       OR lease.expires_at <= clock.database_now)
           )
       )
   )",
    )
    .bind(invocation_id)
    .bind(route_id)
    .bind(revision_id)
    .bind(&owner.host_id)
    .bind(owner.owner_uuid)
    .fetch_optional(pool)
    .await
    .map_err(|_| GatewayExecutionTargetError::Unavailable)
}

#[derive(Debug, FromRow)]
struct ExecutionTargetRow {
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
    handler_contract: String,
    service_instance_id: Option<Uuid>,
    service_instance_fencing_token: Option<i64>,
    session_expires_at: Option<OffsetDateTime>,
    database_now: OffsetDateTime,
}
