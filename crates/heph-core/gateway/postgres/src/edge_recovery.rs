//! Recovery queries for abandoned host-mediated gateway invocations.

use super::SERVICE_RECOVERY_BATCH_SIZE;
use gateway_domain::GatewayEdgeError;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

pub async fn recovery_candidates(
    transaction: &mut Transaction<'_, Postgres>,
    cutoff: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<Vec<Uuid>, GatewayEdgeError> {
    sqlx::query_scalar(
        "SELECT invocation.id
           FROM gateway_invocations AS invocation
           JOIN gateway_revisions AS revision
             ON revision.id = invocation.gateway_revision_id
            AND revision.gateway_id = invocation.gateway_id
           LEFT JOIN gateway_runtime_authority_sessions AS session
             ON session.invocation_id = invocation.id
            AND session.admission_mode = 'host_mediated'
           LEFT JOIN gateway_service_instances AS instance
             ON instance.id = invocation.service_instance_id
            AND instance.gateway_id = invocation.gateway_id
            AND instance.revision_id = invocation.gateway_revision_id
           CROSS JOIN LATERAL (
                SELECT clock_timestamp() AS database_now
           ) AS clock
          WHERE invocation.outcome = 'accepted'
            AND revision.handler_contract = 'http.service.v1'
            AND (
                instance.id IS NULL
                OR instance.fencing_token IS DISTINCT FROM
                   invocation.service_instance_fencing_token
                OR instance.state NOT IN ('ready', 'draining')
                OR instance.lease_expires_at <= clock.database_now
                OR
                session.status IN ('expired', 'revoked')
                OR (session.status = 'active' AND session.expires_at <= $2)
                OR (session.id IS NULL AND invocation.accepted_at <= $1)
            )
          ORDER BY invocation.id
          LIMIT $3
          FOR UPDATE OF invocation SKIP LOCKED",
    )
    .bind(cutoff)
    .bind(now)
    .bind(SERVICE_RECOVERY_BATCH_SIZE)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| GatewayEdgeError::Unavailable)
}

pub async fn recovery_candidate_is_eligible(
    transaction: &mut Transaction<'_, Postgres>,
    invocation_id: Uuid,
    now: OffsetDateTime,
    cutoff: OffsetDateTime,
) -> Result<bool, GatewayEdgeError> {
    sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1
              FROM gateway_invocations AS invocation
              JOIN gateway_revisions AS revision
                ON revision.id = invocation.gateway_revision_id
               AND revision.gateway_id = invocation.gateway_id
              LEFT JOIN gateway_runtime_authority_sessions AS session
                ON session.invocation_id = invocation.id
               AND session.admission_mode = 'host_mediated'
              LEFT JOIN gateway_service_instances AS instance
                ON instance.id = invocation.service_instance_id
               AND instance.gateway_id = invocation.gateway_id
               AND instance.revision_id = invocation.gateway_revision_id
              CROSS JOIN LATERAL (
                   SELECT clock_timestamp() AS database_now
              ) AS clock
             WHERE invocation.id = $1
               AND invocation.outcome = 'accepted'
               AND revision.handler_contract = 'http.service.v1'
               AND (
                   instance.id IS NULL
                   OR instance.fencing_token IS DISTINCT FROM
                      invocation.service_instance_fencing_token
                   OR instance.state NOT IN ('ready', 'draining')
                   OR instance.lease_expires_at <= clock.database_now
                   OR
                   session.status IN ('expired', 'revoked')
                   OR (session.status = 'active' AND session.expires_at <= $2)
                   OR (session.id IS NULL AND invocation.accepted_at <= $3)
               )
        )",
    )
    .bind(invocation_id)
    .bind(now)
    .bind(cutoff)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| GatewayEdgeError::Unavailable)
}
