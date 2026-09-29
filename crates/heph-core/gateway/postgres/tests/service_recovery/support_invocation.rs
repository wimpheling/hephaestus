//! Service recovery invocation and runtime session helpers.

use super::support_types::Fixture;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

pub async fn insert_invocation(
    pool: &sqlx::PgPool,
    fixture: Fixture,
    accepted_at: OffsetDateTime,
) -> Uuid {
    let invocation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_invocations
            (id, gateway_id, gateway_revision_id, gateway_route_id, project_id,
             request_id, outcome, accepted_at,
             service_instance_id, service_instance_fencing_token)
         VALUES ($1, $2, $3, $4, $5, $6, 'accepted', $7, $8, $9)",
    )
    .bind(invocation)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.route)
    .bind(fixture.project)
    .bind(Uuid::new_v4())
    .bind(accepted_at)
    .bind(fixture.service_instance)
    .bind(fixture.service_instance.map(|_| 1_i64))
    .execute(pool)
    .await
    .expect("invocation");
    invocation
}

pub async fn insert_host_session(
    pool: &sqlx::PgPool,
    fixture: Fixture,
    invocation: Uuid,
    now: OffsetDateTime,
    expires_at: OffsetDateTime,
) -> Uuid {
    let snapshot = Uuid::new_v4();
    let session = Uuid::new_v4();
    let hash = [7_u8; 32];
    sqlx::query(
        "INSERT INTO gateway_authorization_snapshots
            (id, invocation_id, gateway_id, gateway_revision_id,
             authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, 'test/v1', $5)",
    )
    .bind(snapshot)
    .bind(invocation)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(hash.as_slice())
    .execute(pool)
    .await
    .expect("snapshot");
    sqlx::query(
        "INSERT INTO gateway_runtime_authority_sessions
            (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
             identity_hash, snapshot_hash, issuance_generation, credential_hash,
             admission_mode, status, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, NULL,
                 'host_mediated', 'active', $8, $9)",
    )
    .bind(session)
    .bind(snapshot)
    .bind(invocation)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(hash.as_slice())
    .bind(hash.as_slice())
    .bind(now - Duration::minutes(10))
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("host session");
    session
}
pub async fn outcome(pool: &sqlx::PgPool, invocation: Uuid) -> String {
    sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
        .bind(invocation)
        .fetch_one(pool)
        .await
        .expect("outcome")
}

pub async fn session_status(pool: &sqlx::PgPool, session: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
        .bind(session)
        .fetch_one(pool)
        .await
        .expect("session status")
}

pub async fn lease_status(pool: &sqlx::PgPool, lease: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE id = $1")
        .bind(lease)
        .fetch_one(pool)
        .await
        .expect("lease status")
}
